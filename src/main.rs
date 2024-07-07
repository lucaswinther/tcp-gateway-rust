mod config;
mod handlers;
mod utils;

use dotenv::dotenv;
use tracing::info;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing_subscriber;
use crate::utils::env::load_env;
use crate::utils::initialization::{init_redis_client, init_metrics_counters, start_servers};
use crate::utils::shutdown::shutdown_signal;
use crate::utils::availability::{ServiceAvailability, start_availability_checkers};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt::init();

    dotenv().ok();
    let (redis_host, redis_key) = load_env();

    info!("Connecting to Redis...");
    let redis_client = init_redis_client(redis_host).await.expect("Failed to Connecting to Redis");
    
    info!("Fetching initial configuration from Redis...");
    let initial_config = config::redis::fetch_config_from_redis(redis_client.clone(), &redis_key)
        .await
        .expect("Failed to load initial config from Redis");
    let config = Arc::new(Mutex::new(initial_config));

    let service_availability = Arc::new(ServiceAvailability::new());
    let redis_client_clone = Arc::clone(&redis_client);
    
    info!("Starting configuration updater...");
    tokio::spawn({
        let config_clone = Arc::clone(&config);
        async move {
            config::redis::config_updater(config_clone, redis_client_clone, &redis_key).await;
        }
    });

    let (total_connections, canary_connections) = init_metrics_counters();

    info!("Starting servers...");
    start_servers(config.clone(), total_connections.clone(), canary_connections.clone(), service_availability.clone()).await?;

    // Start availability checkers after servers are running
    info!("Starting availability checkers...");
    start_availability_checkers(
        config.lock().await.primary_addr.clone(),
        config.lock().await.canary_addr.clone(),
        service_availability.clone(),
    ).await;

    shutdown_signal().await;

    Ok(())
}
