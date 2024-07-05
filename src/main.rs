use axum::{
    routing::get,
    Router,
    response::IntoResponse,
};
use etcd_client::{Client as EtcdClient, ConnectOptions, GetOptions};
use prometheus::{Encoder, TextEncoder, IntCounter, register_int_counter};
use rand::Rng;
use serde::Deserialize;
use std::{
    sync::Arc,
    time::Duration,
};
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::signal;
use tokio::time::timeout;
use tokio::sync::Mutex;
use tracing::{info, error};
use tracing_subscriber;
use dotenv::dotenv;

#[derive(Deserialize, Debug, Clone)]
struct Config {
    primary_addr: String,
    canary_addr: String,
    canary_percentage: u8,
}

async fn fetch_config_from_etcd(client: Arc<Mutex<EtcdClient>>, key: &str) -> Option<Config> {
    let mut client = client.lock().await;
    let resp = client.get(key, Some(GetOptions::new())).await.ok()?;
    if let Some(kv) = resp.kvs().first() {
        let config: Config = serde_json::from_slice(kv.value()).ok()?;
        Some(config)
    } else {
        None
    }
}

async fn handle_connection(
    inbound: TcpStream,
    config: Arc<Mutex<Config>>,
    total_connections: IntCounter,
    canary_connections: IntCounter,
) -> io::Result<()> {
    let target_addr;
    {
        let config = config.lock().await;
        total_connections.inc();

        let mut rng = rand::thread_rng();
        if rng.gen_range(0..100) < config.canary_percentage {
            target_addr = config.canary_addr.clone();
            canary_connections.inc();
            info!(target = "canary", "Routing to canary: {}", target_addr);
        } else {
            target_addr = config.primary_addr.clone();
            info!(target = "primary", "Routing to primary: {}", target_addr);
        }
    }

    let outbound = match timeout(Duration::from_secs(5), TcpStream::connect(&target_addr)).await {
        Ok(Ok(stream)) => Arc::new(Mutex::new(stream)),
        Ok(Err(e)) => {
            error!("Failed to connect to target server: {}", e);
            return Err(e);
        }
        Err(_) => {
            error!("Connection to target server timed out");
            return Err(io::Error::new(io::ErrorKind::TimedOut, "Connection timed out"));
        }
    };

    let inbound = Arc::new(Mutex::new(inbound));
    let inbound_clone = Arc::clone(&inbound);
    let outbound_clone = Arc::clone(&outbound);

    let client_to_server = tokio::spawn(async move {
        let mut inbound = inbound_clone.lock().await;
        let mut outbound = outbound_clone.lock().await;
        if let Err(e) = tokio::io::copy(&mut *inbound, &mut *outbound).await {
            error!("Error in forwarding to outbound: {}", e);
        }
    });

    let inbound_clone = Arc::clone(&inbound);
    let outbound_clone = Arc::clone(&outbound);
    let server_to_client = tokio::spawn(async move {
        let mut inbound = inbound_clone.lock().await;
        let mut outbound = outbound_clone.lock().await;
        if let Err(e) = tokio::io::copy(&mut *outbound, &mut *inbound).await {
            error!("Error in forwarding to inbound: {}", e);
        }
    });

    let _ = tokio::try_join!(client_to_server, server_to_client);

    Ok(())
}

fn encode_metrics() -> Vec<u8> {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    buffer
}

async fn metrics_handler() -> impl IntoResponse {
    let buffer = encode_metrics();
    (
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        buffer,
    )
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending().await;

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("Shutdown signal received");
}

async fn config_updater(config: Arc<Mutex<Config>>, etcd_client: Arc<Mutex<EtcdClient>>, etcd_key: &str) {
    loop {
        if let Some(new_config) = fetch_config_from_etcd(etcd_client.clone(), etcd_key).await {
            let mut current_config = config.lock().await;
            *current_config = new_config;
            info!("Configuration updated from etcd");
        } else {
            error!("Failed to fetch configuration from etcd");
        }
        tokio::time::sleep(Duration::from_secs(30)).await; // Check for updates every 30 seconds
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    tracing_subscriber::fmt::init();

    dotenv().ok();

    let etcd_host = std::env::var("ETCD_HOST").expect("ETCD_HOST must be set");
    let etcd_user = std::env::var("ETCD_USER").expect("ETCD_USER must be set");
    let etcd_password = std::env::var("ETCD_PASSWORD").expect("ETCD_PASSWORD must be set");
    let etcd_key = std::env::var("ETCD_KEY").unwrap_or_else(|_| "gateway_config".to_string());

    let etcd_endpoints = vec![etcd_host];
    let connect_options = Some(ConnectOptions::new().with_user(etcd_user, etcd_password));
    let etcd_client = Arc::new(Mutex::new(EtcdClient::connect(etcd_endpoints, connect_options).await.expect("Failed to connect to etcd")));

    let initial_config = fetch_config_from_etcd(etcd_client.clone(), &etcd_key).await.expect("Failed to load initial config from etcd");
    let config = Arc::new(Mutex::new(initial_config));

    let config_clone = Arc::clone(&config);
    let etcd_client_clone = Arc::clone(&etcd_client);
    tokio::spawn(async move {
        config_updater(config_clone, etcd_client_clone, &etcd_key).await;
    });

    let total_connections = register_int_counter!("total_connections", "Total number of connections").unwrap();
    let canary_connections = register_int_counter!("canary_connections", "Number of connections to the canary instance").unwrap();

    let listener = TcpListener::bind("127.0.0.1:3000").await?;
    info!("TCP Gateway running on 127.0.0.1:3000");

    let app = Router::new().route("/metrics", get(metrics_handler));
    let metrics_addr = tokio::net::TcpListener::bind("127.0.0.1:9090").await.unwrap();    

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
                Ok::<(), io::Error>(())
            }.await {
                error!("Failed to accept connection: {}", e);
            }
        }
    });

    shutdown_signal().await;

    server.abort();
    metrics_server.abort();

    Ok(())
}
