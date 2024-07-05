mod config;
mod handlers;
mod utils;

use dotenv::dotenv;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing_subscriber;
use crate::utils::env::load_env;
use crate::utils::initialization::{init_redis_client, init_metrics_counters, start_servers};
use crate::utils::shutdown::shutdown_signal;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt::init();

    dotenv().ok();
    let (redis_host, redis_key) = load_env();

    let redis_client = init_redis_client(redis_host).await.expect("Failed to load connect Redis");
    let initial_config = config::redis::fetch_config_from_redis(redis_client.clone(), &redis_key).await.expect("Failed to load initial config from Redis");
    let config = Arc::new(Mutex::new(initial_config));

    let config_clone = Arc::clone(&config);
    let redis_client_clone = Arc::clone(&redis_client);
    tokio::spawn(async move {
        config::redis::config_updater(config_clone, redis_client_clone, &redis_key).await;
    });

    let (total_connections, canary_connections) = init_metrics_counters();

    start_servers(config, total_connections, canary_connections).await?;

    shutdown_signal().await;

    Ok(())
}
