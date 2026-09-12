use std::{env, net::SocketAddr};

use ftnl_backend_api::{app, observability, shutdown_signal, AppState};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    const ROUTINE_ID: &str = "ores-routine-DAkC2DkBDXJrWN6Xu5hzr";
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("ftnl_backend_api=info,tower_http=info")),
        )
        .init();

    let telemetry = observability::logger();
    let _ = observability::event(&telemetry, "backend.service.starting")
        .add_trace("ores-trace-0aRlQysBtiGmEIw0W2W35", false)
        .add_routine_id(ROUTINE_ID)
        .send();
    let address: SocketAddr = env::var("FTNL_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse()
        .expect("FTNL_BIND must be a socket address");
    let listener = match TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(error) => {
            let _ = observability::failure(&telemetry, "backend.service.bind_failed")
                .add_trace("ores-trace-iE5rEETi7wLK4iMp5Fh8V", false)
                .add_routine_id(ROUTINE_ID)
                .send();
            let _ = telemetry.close();
            panic!("failed to bind FTNL_BIND: {error:?}");
        }
    };
    info!(%address, "File Tunnel API listening");
    let _ = observability::event(&telemetry, "backend.service.listening")
        .add_trace("ores-trace-1ucUj3sArrJhht1ZX7Ff7", false)
        .add_routine_id(ROUTINE_ID)
        .send();
    let result = axum::serve(listener, app(AppState::default()))
        .with_graceful_shutdown(shutdown_signal())
        .await;
    let _ = observability::event(&telemetry, "backend.service.stopped")
        .add_trace("ores-trace-l95512yZwUUUquRi-3RyU", false)
        .add_routine_id(ROUTINE_ID)
        .send();
    let _ = telemetry.close();
    result.expect("server failed");
}
