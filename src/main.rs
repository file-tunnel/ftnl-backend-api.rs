use std::{env, net::SocketAddr};

use ftnl_backend_api::{app, shutdown_signal, AppState};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("ftnl_backend_api=info,tower_http=info")),
        )
        .init();

    let address: SocketAddr = env::var("FTNL_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse()
        .expect("FTNL_BIND must be a socket address");
    let listener = TcpListener::bind(address)
        .await
        .expect("failed to bind FTNL_BIND");
    info!(%address, "File Tunnel API listening");
    axum::serve(listener, app(AppState::default()))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server failed");
}
