use std::sync::Arc;
use std::time::Duration;
use prometheus::IntCounter;
use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaChaRng;
use tokio::io;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::{info, error};
use crate::config::redis::Config;
use crate::utils::availability::ServiceAvailability;

pub async fn handle_connection(
    inbound: TcpStream,
    config: Arc<Mutex<Config>>,
    total_connections: IntCounter,
    canary_connections: IntCounter,
    availability: Arc<ServiceAvailability>,
) -> io::Result<()> {
    let target_addr;
    {
        let config = config.lock().await;
        total_connections.inc();

        let mut rng = ChaChaRng::from_entropy();
        let canary_available = *availability.canary_available.read().await;
        let primary_available = *availability.primary_available.read().await;

        info!("Canary available: {}, Primary available: {}", canary_available, primary_available);

        if rng.gen_range(0..100) < config.canary_percentage && canary_available {
            target_addr = config.canary_addr.clone();
            canary_connections.inc();
            info!(target = "canary", "Routing to canary: {}", target_addr);
        } else if primary_available {
            target_addr = config.primary_addr.clone();
            info!(target = "primary", "Routing to primary: {}", target_addr);
        } else {
            error!("No available service to route to");
            return Err(io::Error::new(io::ErrorKind::Other, "No available service"));
        }
    }

    info!("Connecting to target address: {}", target_addr);
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
