//! File Tunnel reference API.
//!
//! The storage is intentionally in-process. The capability and lifecycle
//! boundaries are the reusable part; production storage adapters can replace
//! the maps and byte buffers without changing the HTTP contract.

use std::{
    collections::HashMap,
    env,
    sync::Arc,
    time::{Duration, SystemTime},
};

use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use bytes::{Bytes, BytesMut};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::{broadcast, RwLock};
use tower_http::{
    cors::{Any, CorsLayer},
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use uuid::Uuid;

const DEFAULT_MAX_FILES: u16 = 10;
const DEFAULT_MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;
const DEFAULT_EXPIRES_SECONDS: u32 = 600;
const MAX_EXPIRES_SECONDS: u32 = 3600;
const EVENT_TICKET_SECONDS: i64 = 30;

#[derive(Clone)]
pub struct AppState {
    inner: Arc<RwLock<HashMap<Uuid, Tunnel>>>,
    portal_origin: Arc<str>,
}

impl AppState {
    pub fn new(portal_origin: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            portal_origin: Arc::from(portal_origin.into()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new(
            env::var("FTNL_PORTAL_ORIGIN").unwrap_or_else(|_| "http://127.0.0.1:3000".to_owned()),
        )
    }
}

struct Tunnel {
    id: Uuid,
    status: TunnelStatus,
    pairing_hash: Option<[u8; 32]>,
    desktop_hash: [u8; 32],
    phone_hash: Option<[u8; 32]>,
    expires_at: DateTime<Utc>,
    max_files: u16,
    max_file_bytes: u64,
    accept: Vec<String>,
    files: HashMap<Uuid, StoredFile>,
    sequence: u64,
    event_tickets: HashMap<[u8; 32], DateTime<Utc>>,
    events: broadcast::Sender<TunnelEvent>,
}

struct StoredFile {
    descriptor: FileDescriptor,
    content: Option<Bytes>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TunnelStatus {
    Waiting,
    Connected,
    Transferring,
    Complete,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Declared,
    Uploading,
    Available,
    Downloaded,
    Rejected,
    Cancelled,
}

#[derive(Debug, Deserialize)]
pub struct CreateTunnelRequest {
    pub application_id: String,
    #[serde(default = "default_accept")]
    pub accept: Vec<String>,
    #[serde(default = "default_max_files")]
    pub max_files: u16,
    #[serde(default = "default_max_file_bytes")]
    pub max_file_bytes: u64,
    #[serde(default = "default_expires_seconds")]
    pub expires_in_seconds: u32,
}

#[derive(Debug, Serialize)]
pub struct CreateTunnelResponse {
    pub api_version: &'static str,
    pub tunnel_id: Uuid,
    pub status: TunnelStatus,
    pub pairing_uri: String,
    pub desktop_capability: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ClaimTunnelRequest {
    pub pairing_secret: String,
    pub device_label: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ClaimTunnelResponse {
    pub phone_capability: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct DeclareFileRequest {
    pub name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub last_modified_ms: Option<u64>,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileDescriptor {
    pub file_id: Uuid,
    pub name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub bytes_transferred: u64,
    pub status: FileStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct TunnelSnapshot {
    pub tunnel_id: Uuid,
    pub status: TunnelStatus,
    pub expires_at: DateTime<Utc>,
    pub files: Vec<FileDescriptor>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TunnelEvent {
    pub event_id: Uuid,
    pub sequence: u64,
    pub occurred_at: DateTime<Utc>,
    pub tunnel_id: Uuid,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_transferred: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct EventTicketResponse {
    pub ticket: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct EventTicketQuery {
    ticket: String,
}

#[derive(Debug, Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

#[derive(Debug, thiserror::Error)]
enum ApiError {
    #[error("invalid request: {0}")]
    Invalid(&'static str),
    #[error("capability is missing or invalid")]
    Unauthorized,
    #[error("tunnel or file was not found")]
    NotFound,
    #[error("tunnel has expired")]
    Expired,
    #[error("resource state conflicts with this operation")]
    Conflict,
    #[error("declared upload is too large")]
    TooLarge,
    #[error("media type is not accepted")]
    UnsupportedMedia,
}

#[derive(Serialize)]
struct Problem {
    #[serde(rename = "type")]
    kind: String,
    title: &'static str,
    status: u16,
    detail: String,
    code: &'static str,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, title, code) = match self {
            Self::Invalid(_) => (
                StatusCode::BAD_REQUEST,
                "Invalid request",
                "invalid_request",
            ),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "Unauthorized capability",
                "unauthorized",
            ),
            Self::NotFound => (StatusCode::NOT_FOUND, "Not found", "not_found"),
            Self::Expired => (StatusCode::GONE, "Tunnel expired", "tunnel_expired"),
            Self::Conflict => (StatusCode::CONFLICT, "State conflict", "state_conflict"),
            Self::TooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "File too large",
                "file_too_large",
            ),
            Self::UnsupportedMedia => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Unsupported media type",
                "unsupported_media_type",
            ),
        };
        let problem = Problem {
            kind: format!("https://file-tunnel.dev/problems/{code}"),
            title,
            status: status.as_u16(),
            detail: self.to_string(),
            code,
        };
        (
            status,
            [(header::CONTENT_TYPE, "application/problem+json")],
            Json(problem),
        )
            .into_response()
    }
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/tunnels", post(create_tunnel))
        .route(
            "/v1/tunnels/{tunnel_id}",
            get(get_tunnel).delete(cancel_tunnel),
        )
        .route("/v1/tunnels/{tunnel_id}/claim", post(claim_tunnel))
        .route(
            "/v1/tunnels/{tunnel_id}/event-tickets",
            post(create_event_ticket),
        )
        .route("/v1/tunnels/{tunnel_id}/events", get(connect_events))
        .route("/v1/tunnels/{tunnel_id}/files", post(declare_file))
        .route(
            "/v1/tunnels/{tunnel_id}/files/{file_id}/content",
            put(upload_file).get(download_file),
        )
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::new(
            header::HeaderName::from_static("x-request-id"),
            MakeRequestUuid,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_headers([
                    header::AUTHORIZATION,
                    header::CONTENT_TYPE,
                    header::HeaderName::from_static("idempotency-key"),
                ])
                .allow_methods(Any),
        )
        .with_state(state)
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn create_tunnel(
    State(state): State<AppState>,
    Json(request): Json<CreateTunnelRequest>,
) -> Result<(StatusCode, Json<CreateTunnelResponse>), ApiError> {
    validate_create(&request)?;
    let tunnel_id = Uuid::new_v4();
    let pairing_secret = token();
    let desktop_capability = token();
    let expires_at = Utc::now() + chrono::Duration::seconds(i64::from(request.expires_in_seconds));
    let (events, _) = broadcast::channel(256);
    let tunnel = Tunnel {
        id: tunnel_id,
        status: TunnelStatus::Waiting,
        pairing_hash: Some(token_hash(&pairing_secret)),
        desktop_hash: token_hash(&desktop_capability),
        phone_hash: None,
        expires_at,
        max_files: request.max_files,
        max_file_bytes: request.max_file_bytes,
        accept: request.accept,
        files: HashMap::new(),
        sequence: 0,
        event_tickets: HashMap::new(),
        events,
    };
    state.inner.write().await.insert(tunnel_id, tunnel);
    let origin = state.portal_origin.trim_end_matches('/');
    let pairing_uri = format!("{origin}/t/{tunnel_id}#c={pairing_secret}");
    Ok((
        StatusCode::CREATED,
        Json(CreateTunnelResponse {
            api_version: "v1",
            tunnel_id,
            status: TunnelStatus::Waiting,
            pairing_uri,
            desktop_capability,
            expires_at,
        }),
    ))
}

async fn claim_tunnel(
    State(state): State<AppState>,
    Path(tunnel_id): Path<Uuid>,
    Json(request): Json<ClaimTunnelRequest>,
) -> Result<Json<ClaimTunnelResponse>, ApiError> {
    let _ = request.device_label.as_deref();
    let supplied = token_hash(&request.pairing_secret);
    let mut tunnels = state.inner.write().await;
    let tunnel = tunnels.get_mut(&tunnel_id).ok_or(ApiError::NotFound)?;
    ensure_active(tunnel)?;
    let expected = tunnel.pairing_hash.ok_or(ApiError::Conflict)?;
    if !hashes_equal(&expected, &supplied) {
        return Err(ApiError::Unauthorized);
    }
    let phone_capability = token();
    tunnel.phone_hash = Some(token_hash(&phone_capability));
    tunnel.pairing_hash = None;
    tunnel.status = TunnelStatus::Connected;
    publish(tunnel, "tunnel.connected", None, None, None);
    Ok(Json(ClaimTunnelResponse {
        phone_capability,
        expires_at: tunnel.expires_at,
    }))
}

async fn get_tunnel(
    State(state): State<AppState>,
    Path(tunnel_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<TunnelSnapshot>, ApiError> {
    let supplied = bearer_hash(&headers)?;
    let tunnels = state.inner.read().await;
    let tunnel = tunnels.get(&tunnel_id).ok_or(ApiError::NotFound)?;
    ensure_active(tunnel)?;
    require_either(tunnel, &supplied)?;
    let mut files: Vec<_> = tunnel
        .files
        .values()
        .map(|stored| stored.descriptor.clone())
        .collect();
    files.sort_by_key(|file| file.created_at);
    Ok(Json(TunnelSnapshot {
        tunnel_id,
        status: tunnel.status,
        expires_at: tunnel.expires_at,
        files,
    }))
}

async fn cancel_tunnel(
    State(state): State<AppState>,
    Path(tunnel_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let supplied = bearer_hash(&headers)?;
    let mut tunnels = state.inner.write().await;
    let tunnel = tunnels.get_mut(&tunnel_id).ok_or(ApiError::NotFound)?;
    require_desktop(tunnel, &supplied)?;
    tunnel.status = TunnelStatus::Cancelled;
    tunnel.files.clear();
    publish(tunnel, "tunnel.cancelled", None, None, None);
    Ok(StatusCode::NO_CONTENT)
}

async fn declare_file(
    State(state): State<AppState>,
    Path(tunnel_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<DeclareFileRequest>,
) -> Result<(StatusCode, Json<FileDescriptor>), ApiError> {
    let supplied = bearer_hash(&headers)?;
    validate_filename(&request.name)?;
    let mut tunnels = state.inner.write().await;
    let tunnel = tunnels.get_mut(&tunnel_id).ok_or(ApiError::NotFound)?;
    ensure_active(tunnel)?;
    require_phone(tunnel, &supplied)?;
    if tunnel.files.len() >= usize::from(tunnel.max_files) {
        return Err(ApiError::TooLarge);
    }
    if request.size_bytes > tunnel.max_file_bytes {
        return Err(ApiError::TooLarge);
    }
    if !media_is_accepted(&tunnel.accept, &request.media_type) {
        return Err(ApiError::UnsupportedMedia);
    }
    if let Some(digest) = &request.sha256 {
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ApiError::Invalid(
                "sha256 must be 64 hexadecimal characters",
            ));
        }
    }
    let _ = request.last_modified_ms;
    let descriptor = FileDescriptor {
        file_id: Uuid::new_v4(),
        name: request.name,
        media_type: request.media_type,
        size_bytes: request.size_bytes,
        bytes_transferred: 0,
        status: FileStatus::Declared,
        created_at: Utc::now(),
    };
    tunnel.files.insert(
        descriptor.file_id,
        StoredFile {
            descriptor: descriptor.clone(),
            content: None,
        },
    );
    publish(
        tunnel,
        "file.declared",
        Some(descriptor.file_id),
        Some(0),
        None,
    );
    Ok((StatusCode::CREATED, Json(descriptor)))
}

async fn upload_file(
    State(state): State<AppState>,
    Path((tunnel_id, file_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    body: Body,
) -> Result<StatusCode, ApiError> {
    let supplied = bearer_hash(&headers)?;
    let expected_size = {
        let mut tunnels = state.inner.write().await;
        let tunnel = tunnels.get_mut(&tunnel_id).ok_or(ApiError::NotFound)?;
        ensure_active(tunnel)?;
        require_phone(tunnel, &supplied)?;
        let stored = tunnel.files.get_mut(&file_id).ok_or(ApiError::NotFound)?;
        if stored.content.is_some() {
            return Err(ApiError::Conflict);
        }
        stored.descriptor.status = FileStatus::Uploading;
        stored.descriptor.size_bytes
    };

    let mut bytes = BytesMut::with_capacity(expected_size.min(8 * 1024 * 1024) as usize);
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ApiError::Invalid("invalid request body"))?;
        let next_size = bytes.len().saturating_add(chunk.len());
        if next_size as u64 > expected_size {
            return Err(ApiError::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
        let mut tunnels = state.inner.write().await;
        if let Some(tunnel) = tunnels.get_mut(&tunnel_id) {
            if let Some(stored) = tunnel.files.get_mut(&file_id) {
                stored.descriptor.bytes_transferred = next_size as u64;
            }
            publish(
                tunnel,
                "file.progress",
                Some(file_id),
                Some(next_size as u64),
                None,
            );
        }
    }
    if bytes.len() as u64 != expected_size {
        return Err(ApiError::Conflict);
    }

    let mut tunnels = state.inner.write().await;
    let tunnel = tunnels.get_mut(&tunnel_id).ok_or(ApiError::NotFound)?;
    let stored = tunnel.files.get_mut(&file_id).ok_or(ApiError::NotFound)?;
    stored.content = Some(bytes.freeze());
    stored.descriptor.status = FileStatus::Available;
    tunnel.status = TunnelStatus::Transferring;
    publish(
        tunnel,
        "file.available",
        Some(file_id),
        Some(expected_size),
        None,
    );
    Ok(StatusCode::NO_CONTENT)
}

async fn download_file(
    State(state): State<AppState>,
    Path((tunnel_id, file_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let supplied = bearer_hash(&headers)?;
    let mut tunnels = state.inner.write().await;
    let tunnel = tunnels.get_mut(&tunnel_id).ok_or(ApiError::NotFound)?;
    ensure_active(tunnel)?;
    require_desktop(tunnel, &supplied)?;
    let stored = tunnel.files.get_mut(&file_id).ok_or(ApiError::NotFound)?;
    let content = stored.content.clone().ok_or(ApiError::Conflict)?;
    let media_type = HeaderValue::from_str(&stored.descriptor.media_type)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    stored.descriptor.status = FileStatus::Downloaded;
    publish(
        tunnel,
        "file.downloaded",
        Some(file_id),
        Some(content.len() as u64),
        None,
    );
    Ok(([(header::CONTENT_TYPE, media_type)], content).into_response())
}

async fn create_event_ticket(
    State(state): State<AppState>,
    Path(tunnel_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<EventTicketResponse>), ApiError> {
    let supplied = bearer_hash(&headers)?;
    let mut tunnels = state.inner.write().await;
    let tunnel = tunnels.get_mut(&tunnel_id).ok_or(ApiError::NotFound)?;
    ensure_active(tunnel)?;
    require_either(tunnel, &supplied)?;
    let ticket = token();
    let expires_at = Utc::now() + chrono::Duration::seconds(EVENT_TICKET_SECONDS);
    tunnel.event_tickets.insert(token_hash(&ticket), expires_at);
    Ok((
        StatusCode::CREATED,
        Json(EventTicketResponse { ticket, expires_at }),
    ))
}

async fn connect_events(
    State(state): State<AppState>,
    Path(tunnel_id): Path<Uuid>,
    Query(query): Query<EventTicketQuery>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let mut tunnels = state.inner.write().await;
    let tunnel = tunnels.get_mut(&tunnel_id).ok_or(ApiError::NotFound)?;
    ensure_active(tunnel)?;
    let supplied = token_hash(&query.ticket);
    let ticket_hash = tunnel
        .event_tickets
        .keys()
        .find(|hash| hashes_equal(hash, &supplied))
        .copied()
        .ok_or(ApiError::Unauthorized)?;
    let expiry = tunnel
        .event_tickets
        .remove(&ticket_hash)
        .ok_or(ApiError::Unauthorized)?;
    if expiry <= Utc::now() {
        return Err(ApiError::Unauthorized);
    }
    let receiver = tunnel.events.subscribe();
    Ok(websocket
        .on_upgrade(move |socket| event_socket(socket, receiver))
        .into_response())
}

async fn event_socket(mut socket: WebSocket, mut events: broadcast::Receiver<TunnelEvent>) {
    while let Ok(event) = events.recv().await {
        let Ok(payload) = serde_json::to_string(&event) else {
            continue;
        };
        if socket.send(Message::Text(payload.into())).await.is_err() {
            break;
        }
    }
    let _ = socket.close().await;
}

fn validate_create(request: &CreateTunnelRequest) -> Result<(), ApiError> {
    if request.application_id.is_empty() || request.application_id.len() > 128 {
        return Err(ApiError::Invalid("application_id must be 1..128 bytes"));
    }
    if request.max_files == 0 || request.max_files > 100 {
        return Err(ApiError::Invalid("max_files must be between 1 and 100"));
    }
    if request.max_file_bytes == 0 || request.max_file_bytes > 5 * 1024 * 1024 * 1024 {
        return Err(ApiError::Invalid(
            "max_file_bytes is outside the supported range",
        ));
    }
    if !(60..=MAX_EXPIRES_SECONDS).contains(&request.expires_in_seconds) {
        return Err(ApiError::Invalid("expires_in_seconds must be 60..3600"));
    }
    if request.accept.is_empty() || request.accept.len() > 32 {
        return Err(ApiError::Invalid(
            "accept must contain 1..32 media patterns",
        ));
    }
    Ok(())
}

fn validate_filename(name: &str) -> Result<(), ApiError> {
    if name.is_empty() || name.len() > 255 || name.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(ApiError::Invalid(
            "name must be printable and at most 255 bytes",
        ));
    }
    Ok(())
}

fn ensure_active(tunnel: &Tunnel) -> Result<(), ApiError> {
    if tunnel.expires_at <= Utc::now() || tunnel.status == TunnelStatus::Expired {
        return Err(ApiError::Expired);
    }
    if tunnel.status == TunnelStatus::Cancelled {
        return Err(ApiError::Conflict);
    }
    Ok(())
}

fn media_is_accepted(patterns: &[String], media_type: &str) -> bool {
    patterns.iter().any(|pattern| {
        pattern == "*/*"
            || pattern == media_type
            || pattern
                .strip_suffix("/*")
                .is_some_and(|prefix| media_type.starts_with(&format!("{prefix}/")))
    })
}

fn bearer_hash(headers: &HeaderMap) -> Result<[u8; 32], ApiError> {
    let value = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty())
        .ok_or(ApiError::Unauthorized)?;
    Ok(token_hash(value))
}

fn require_desktop(tunnel: &Tunnel, supplied: &[u8; 32]) -> Result<(), ApiError> {
    hashes_equal(&tunnel.desktop_hash, supplied)
        .then_some(())
        .ok_or(ApiError::Unauthorized)
}

fn require_phone(tunnel: &Tunnel, supplied: &[u8; 32]) -> Result<(), ApiError> {
    tunnel
        .phone_hash
        .as_ref()
        .is_some_and(|expected| hashes_equal(expected, supplied))
        .then_some(())
        .ok_or(ApiError::Unauthorized)
}

fn require_either(tunnel: &Tunnel, supplied: &[u8; 32]) -> Result<(), ApiError> {
    if hashes_equal(&tunnel.desktop_hash, supplied)
        || tunnel
            .phone_hash
            .as_ref()
            .is_some_and(|expected| hashes_equal(expected, supplied))
    {
        Ok(())
    } else {
        Err(ApiError::Unauthorized)
    }
}

fn publish(
    tunnel: &mut Tunnel,
    kind: &'static str,
    file_id: Option<Uuid>,
    bytes_transferred: Option<u64>,
    reason_code: Option<&'static str>,
) {
    tunnel.sequence += 1;
    let _ = tunnel.events.send(TunnelEvent {
        event_id: Uuid::new_v4(),
        sequence: tunnel.sequence,
        occurred_at: Utc::now(),
        tunnel_id: tunnel.id,
        kind,
        file_id,
        bytes_transferred,
        reason_code,
    });
}

fn token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn token_hash(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

fn hashes_equal(left: &[u8; 32], right: &[u8; 32]) -> bool {
    bool::from(left.ct_eq(right))
}

fn default_accept() -> Vec<String> {
    vec!["image/*".to_owned()]
}

const fn default_max_files() -> u16 {
    DEFAULT_MAX_FILES
}

const fn default_max_file_bytes() -> u64 {
    DEFAULT_MAX_FILE_BYTES
}

const fn default_expires_seconds() -> u32 {
    DEFAULT_EXPIRES_SECONDS
}

pub async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

pub fn token_ttl_hint() -> Duration {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|_| Duration::from_secs(DEFAULT_EXPIRES_SECONDS.into()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_opaque_and_hash_comparison_is_constant_time() {
        let value = token();
        assert_eq!(value.len(), 64);
        let digest = token_hash(&value);
        assert!(hashes_equal(&digest, &token_hash(&value)));
        assert!(!hashes_equal(&digest, &token_hash("wrong")));
    }

    #[test]
    fn media_patterns_are_explicit() {
        let patterns = vec!["image/*".to_owned(), "application/pdf".to_owned()];
        assert!(media_is_accepted(&patterns, "image/jpeg"));
        assert!(media_is_accepted(&patterns, "application/pdf"));
        assert!(!media_is_accepted(&patterns, "text/html"));
    }

    #[test]
    fn filenames_cannot_hide_control_characters() {
        assert!(validate_filename("IMG_0001.jpg").is_ok());
        assert!(validate_filename("../IMG_0001.jpg").is_ok());
        assert!(validate_filename("bad\nname.jpg").is_err());
    }

    #[tokio::test]
    async fn create_response_keeps_pairing_secret_in_fragment() {
        let state = AppState::new("https://upload.file-tunnel.dev");
        let (_, Json(response)) = create_tunnel(
            State(state),
            Json(CreateTunnelRequest {
                application_id: "tests".to_owned(),
                accept: default_accept(),
                max_files: default_max_files(),
                max_file_bytes: default_max_file_bytes(),
                expires_in_seconds: default_expires_seconds(),
            }),
        )
        .await
        .unwrap();
        assert!(response.pairing_uri.contains("#c="));
        assert!(!response.pairing_uri.contains("?c="));
    }
}
