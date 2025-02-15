use serde::Deserialize;
use tokio::sync::RwLock;
use std::sync::Arc;
use tokio::time::{self, Duration};
use redis::AsyncCommands;
use tracing::{info, error};
use tokio::net::TcpStream;
use tokio::time::timeout;

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct GatewayConfig {
    pub primary_addr: String,
    pub canary_addr: String,
    pub canary_percentage: u8,
    #[serde(default = "default_interval_config_update")]
    config_update_interval: u64,
    #[serde(default = "default_interval_status_update")]
    status_update_interval: u64,
}

fn default_interval_config_update() -> u64 {
    5
}

fn default_interval_status_update() -> u64 {
    1
}


#[derive(Debug, Clone, PartialEq)]
pub struct ServerStatus {
    pub primary_online: bool,
    pub canary_online: bool,
}

#[derive(Clone)]
pub struct ConfigManager {
    config: Arc<RwLock<GatewayConfig>>,
    server_status: Arc<RwLock<ServerStatus>>,
    redis_client: redis::Client,
}

impl ConfigManager {
    // Cria uma nova instância do gerenciador de configuração
    pub async fn new(redis_url: &str) -> Self {
        let client = redis::Client::open(redis_url).expect("Invalid Redis URL");
        let config = Arc::new(RwLock::new(GatewayConfig {
            primary_addr: String::new(),
            canary_addr: String::new(),
            canary_percentage: 0,
            config_update_interval: default_interval_config_update(),
            status_update_interval: default_interval_status_update(),
        }));
        let server_status = Arc::new(RwLock::new(ServerStatus {
            primary_online: false,
            canary_online: false,
        }));
        ConfigManager {
            config,
            server_status,
            redis_client: client
        }
    }

    // Atualiza a configuração lendo os dados do Redis
    pub async fn update_config(&self) {
        let mut con = match self.redis_client.get_multiplexed_async_connection().await {
            Ok(conn) => conn,
            Err(e) => {
                error!("Failed to connect to Redis: {}", e);
                return;
            }
        };
        let config_str: String = match con.get("gateway_config").await {
            Ok(config) => config,
            Err(e) => {
                error!("Failed to get config from Redis: {}", e);
                return;
            }
        };
        let new_config: GatewayConfig = match serde_json::from_str::<GatewayConfig>(&config_str) {
            Ok(mut config) => {
                if config.config_update_interval == 0 {
                    config.config_update_interval = default_interval_config_update();
                }
                if config.status_update_interval == 0 {
                    config.status_update_interval = default_interval_status_update();
                }
                config
            },
            Err(e) => {
                error!("Failed to deserialize config: {}", e);
                return;
            }
        };

        let mut config_write = self.config.write().await;
        if *config_write != new_config {
            *config_write = new_config;
            info!("Updated configuration: {:?}", *config_write);
        }
    }

    // Inicia o loop de atualização da configuração
    pub async fn start_update_config_loop(self, interval: Duration) {
        let mut interval = time::interval(interval);

        loop {
            interval.tick().await;
            self.update_config().await;
        }
    }

    // Inicia o loop de verificação dos servidores
    pub async fn start_update_server_status_loop(self, interval: Duration) {
        let mut interval = time::interval(interval);

        loop {
            interval.tick().await;
            self.update_server_status().await;
        }
    }

    // Verifica o status dos servidores e atualiza o estado
    async fn update_server_status(&self) {
        let config = self.config.read().await.clone();
        let primary_online = check_server(&config.primary_addr).await;
        let canary_online = check_server(&config.canary_addr).await;

        let new_status = ServerStatus {
            primary_online,
            canary_online,
        };

        let mut status_write = self.server_status.write().await;
        if *status_write != new_status {
            *status_write = new_status;
            info!("Server status updated: {:?}", *status_write);
        }
    }

    // Retorna a configuração atual
    pub async fn get_config(&self) -> GatewayConfig {
        self.config.read().await.clone()
    }

    // Retorna o status dos servidores
    pub async fn get_server_status(&self) -> ServerStatus {
        self.server_status.read().await.clone()
    }
}

// Função auxiliar para verificar se um servidor está online
async fn check_server(addr: &str) -> bool {
    let timeout_duration = Duration::from_secs(2);
    match timeout(timeout_duration, TcpStream::connect(addr)).await {
        Ok(Ok(_)) => true,
        _ => false,
    }
}
