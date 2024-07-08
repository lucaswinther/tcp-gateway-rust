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

#[tokio::main]
async fn main() {
    // Carrega as variáveis de ambiente do arquivo .env
    dotenv().ok();
    
    // Inicializa o sistema de logging
    tracing_subscriber::fmt::init();

    // Obtém a URL do Redis e a chave de configuração do ambiente
    let redis_url = env::var("REDIS_URL").expect("REDIS_URL must be set in .env file");
    let redis_key = env::var("REDIS_KEY").expect("REDIS_KEY must be set in .env file");
    let config_update_interval = env::var("CONFIG_UPDATE_INTERVAL")
        .unwrap_or_else(|_| "10".to_string())
        .parse::<u64>()
        .expect("CONFIG_UPDATE_INTERVAL must be a valid number");
    let status_update_interval = env::var("STATUS_UPDATE_INTERVAL")
        .unwrap_or_else(|_| "10".to_string())
        .parse::<u64>()
        .expect("STATUS_UPDATE_INTERVAL must be a valid number");

    // Cria o gerenciador de configuração
    let config_manager = ConfigManager::new(&redis_url, &redis_key).await;

    // Inicia o loop de atualização da configuração
    let config_manager_clone = config_manager.clone();
    spawn(async move {
        config_manager_clone.start_update_config_loop(Duration::from_secs(config_update_interval)).await;
    });

    // Inicia o loop de verificação de status dos servidores
    let config_manager_clone = config_manager.clone();
    spawn(async move {
        config_manager_clone.start_update_server_status_loop(Duration::from_secs(status_update_interval)).await;
    });

    // Inicia o balanceador de carga
    let balancer = Balancer::new(config_manager);
    info!("Starting the balancer on 0.0.0.0:8080");
    if let Err(e) = balancer.run("0.0.0.0:8080").await {
        error!("Failed to run the balancer: {}", e);
    }
}
