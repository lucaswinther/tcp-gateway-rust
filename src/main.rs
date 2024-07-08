mod config;
mod balancer;

use crate::config::ConfigManager;
use crate::balancer::Balancer;
use tokio::spawn;
use dotenv::dotenv;
use std::env;
use tracing::{info, error};
use tracing_subscriber;
use std::time::Duration;

struct EnvConfig {
    redis_url: String,
    config_update_interval: Duration,
    status_update_interval: Duration,
}

fn init_env() -> EnvConfig {
    dotenv().ok();

    let redis_url = env::var("REDIS_URL").expect("REDIS_URL must be set in .env file");
    let config_update_interval = env::var("CONFIG_UPDATE_INTERVAL")
        .unwrap_or_else(|_| "10".to_string())
        .parse::<u64>()
        .expect("CONFIG_UPDATE_INTERVAL must be a valid number");
    let status_update_interval = env::var("STATUS_UPDATE_INTERVAL")
        .unwrap_or_else(|_| "10".to_string())
        .parse::<u64>()
        .expect("STATUS_UPDATE_INTERVAL must be a valid number");

    EnvConfig {
        redis_url,
        config_update_interval: Duration::from_secs(config_update_interval),
        status_update_interval: Duration::from_secs(status_update_interval),
    }
}

#[tokio::main]
async fn main() {
    // Inicializa o sistema de logging
    tracing_subscriber::fmt::init();

    // Inicializa as variáveis de ambiente
    let env_config = init_env();

    // Cria o gerenciador de configuração
    let config_manager = ConfigManager::new(&env_config.redis_url).await;

    // Inicia o loop de atualização da configuração
    let config_manager_clone = config_manager.clone();
    spawn(async move {
        config_manager_clone.start_update_config_loop(env_config.config_update_interval).await;
    });

    // Inicia o loop de verificação de status dos servidores
    let config_manager_clone = config_manager.clone();
    spawn(async move {
        config_manager_clone.start_update_server_status_loop(env_config.status_update_interval).await;
    });

    // Inicia o balanceador de carga
    let balancer = Balancer::new(config_manager);
    info!("Starting the balancer on 0.0.0.0:8080");
    if let Err(e) = balancer.run("0.0.0.0:8080").await {
        error!("Failed to run the balancer: {}", e);
    }
}
