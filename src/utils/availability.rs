use std::sync::Arc;
use tokio::sync::{RwLock};
use tokio::time::{self, Duration};
use tracing::{info, error};
use tokio::net::TcpStream;

pub struct ServiceAvailability {
    pub primary_available: RwLock<bool>,
    pub canary_available: RwLock<bool>,
}

impl ServiceAvailability {
    pub fn new() -> Self {
        Self {
            primary_available: RwLock::new(false),
            canary_available: RwLock::new(false),
        }
    }

    pub async fn update_availability(&self, addr: &str, available: bool) {
        let mut availability = if addr.contains("primary") {
            self.primary_available.write().await
        } else {
            self.canary_available.write().await
        };

        *availability = available;
        info!("Service {} is now {}", addr, if available { "available" } else { "unavailable" });
    }
}

async fn check_service_availability(addr: String, availability: Arc<ServiceAvailability>) {
    loop {
        let available = TcpStream::connect(&addr).await.is_ok();
        info!("Checking availability for {}: {}", addr, available);
        availability.update_availability(&addr, available).await;
        time::sleep(Duration::from_secs(5)).await; // Check every 10 seconds
    }
}

pub async fn start_availability_checkers(
    primary_addr: String,
    canary_addr: String,
    availability: Arc<ServiceAvailability>,
) {
    info!("Starting availability checkers for primary: {} and canary: {}", primary_addr, canary_addr);
    tokio::spawn(check_service_availability(primary_addr, availability.clone()));
    tokio::spawn(check_service_availability(canary_addr, availability.clone()));
}
