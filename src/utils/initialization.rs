use std::sync::Arc;
use axum::routing::get;
use tokio::sync::Mutex;
use redis::Client;
use prometheus::{IntCounter, register_int_counter};
use axum::Router;
use tokio::net::TcpListener;
use tracing::{info, error};
use crate::config::redis::Config;
use crate::handlers::connection::handle_connection;
use crate::handlers::metrics::metrics_handler;

pub async fn init_redis_client(redis_host: String) -> Result<Arc<Mutex<Client>>, redis::RedisError> {
    let client = Client::open(redis_host)?;
    Ok(Arc::new(Mutex::new(client)))
}

pub fn init_metrics_counters() -> (IntCounter, IntCounter) {
    let total_connections = register_int_counter!("total_connections", "Total number of connections").unwrap();
    let canary_connections = register_int_counter!("canary_connections", "Number of connections to the canary instance").unwrap();
    (total_connections, canary_connections)
}

pub async fn start_servers(
    config: Arc<Mutex<Config>>,
    total_connections: IntCounter,
    canary_connections: IntCounter,
) -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:3000").await?;
    info!("TCP Gateway running on 127.0.0.1:3000");

    let app = Router::new().route("/metrics", get(metrics_handler));
    let metrics_addr = TcpListener::bind("127.0.0.1:9090").await.unwrap();

    let metrics_server = tokio::spawn(async move {
        axum::serve(metrics_addr, app).await.unwrap();
    });

    let server = tokio::spawn(async move {
        loop {
            if let Err(e) = async {
                let (socket, _) = listener.accept().await?;
                let config = Arc::clone(&config);
                let total_connections = total_connections.clone();
                let canary_connections = canary_connections.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(socket, config, total_connections, canary_connections).await {
                        error!("Failed to handle connection: {}", e);
                    }
                });
                Ok::<(), std::io::Error>(())
            }.await {
                error!("Failed to accept connection: {}", e);
            }
        }
    });

    tokio::select! {
        _ = metrics_server => {},
        _ = server => {},
    }

    Ok(())
}
