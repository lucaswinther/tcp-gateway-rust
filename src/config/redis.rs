use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, error};
use redis::{AsyncCommands, Client};

#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    pub primary_addr: String,
    pub canary_addr: String,
    pub canary_percentage: u8,
}

pub async fn fetch_config_from_redis(client: Arc<Mutex<Client>>, key: &str) -> Option<Config> {
    let mut conn = {
        let client = client.lock().await;
        match client.get_tokio_connection().await {
            Ok(conn) => conn,
            Err(err) => {
                error!("Failed to get async connection: {}", err);
                return None;
            }
        }
    };

    let config_str: String = match conn.get(key).await {
        Ok(val) => val,
        Err(err) => {
            error!("Failed to get value from Redis: {}", err);
            return None;
        }
    };

    serde_json::from_str(&config_str).ok()
}

pub async fn config_updater(config: Arc<Mutex<Config>>, client: Arc<Mutex<Client>>, key: &str) {
    loop {
        if let Some(new_config) = fetch_config_from_redis(client.clone(), key).await {
            let mut current_config = config.lock().await;
            *current_config = new_config;
            info!("Configuration updated from Redis");
        } else {
            error!("Failed to fetch configuration from Redis");
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await; // Check for updates every 30 seconds
    }
}
