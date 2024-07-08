use std::net::ToSocketAddrs;

use rand::Rng;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crate::config::{ConfigManager, ServerStatus};
use tracing::{info, error, debug, warn};
use tokio::time::{self, Instant};

pub struct Balancer {
    config_manager: ConfigManager,
}

impl Balancer {
    // Cria uma nova instância do balanceador de carga
    pub fn new(config_manager: ConfigManager) -> Self {
        Balancer { config_manager }
    }

    // Inicia o balanceador de carga e escuta por conexões TCP
    pub async fn run(&self, listen_addr: &str) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(listen_addr).await?;
        info!("Balancer running on {}", listen_addr);

        loop {
            let (socket, addr) = listener.accept().await?;
            info!("Accepted connection from {}", addr);

            let config_manager = self.config_manager.clone();

            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(socket, config_manager).await {
                    error!("Error handling connection: {}", e);
                }
            });
        }
    }

    // Manipula uma conexão recebida, encaminhando a requisição para o servidor de destino e retornando a resposta
    async fn handle_connection(socket: TcpStream, config_manager: ConfigManager) -> Result<(), Box<dyn std::error::Error>> {
        let start = Instant::now(); // Inicia a medição do tempo
        let (mut reader, mut writer) = socket.into_split();
        let mut buf = vec![0; 4096];

        // Lê a requisição do cliente
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }

        let request = String::from_utf8_lossy(&buf[..n]).to_string();
        debug!("Received request: {}", request);

        // Obtém a configuração atual e o status dos servidores
        let config = config_manager.get_config().await;
        let server_status = config_manager.get_server_status().await;

        let target_addr = if server_status.primary_online && server_status.canary_online {
            // Se ambos os servidores estiverem online, use a porcentagem configurada para decidir
            if rand::thread_rng().gen_range(0..100) < config.canary_percentage {
                &config.canary_addr
            } else {
                &config.primary_addr
            }
        } else if server_status.primary_online {
            &config.primary_addr
        } else if server_status.canary_online {
            &config.canary_addr
        } else {
            // Se nenhum servidor estiver online, retorne uma resposta de erro
            let error_message = "HTTP/1.1 503 Service Unavailable\r\n\r\nNo available servers";
            writer.write_all(error_message.as_bytes()).await?;
            writer.flush().await?;
            error!("No available servers. Sent 503 Service Unavailable to client.");
            return Ok(());
        };

        info!("Forwarding request to target address: {}", target_addr);

        // Conecta ao servidor de destino e encaminha a requisição
        let target_socket = match TcpStream::connect(target_addr.to_socket_addrs()?.next().unwrap()).await {
            Ok(stream) => stream,
            Err(e) => {
                error!("Failed to connect to target server {}: {}", target_addr, e);
                let error_message = "HTTP/1.1 502 Bad Gateway\r\n\r\nFailed to connect to target server";
                writer.write_all(error_message.as_bytes()).await?;
                writer.flush().await?;
                return Ok(());
            }
        };
        let (mut target_reader, mut target_writer) = target_socket.into_split();

        target_writer.write_all(request.as_bytes()).await?;
        target_writer.flush().await?;

        // Lê a resposta do servidor de destino
        let n = match target_reader.read(&mut buf).await {
            Ok(n) if n == 0 => {
                warn!("Target server {} closed the connection", target_addr);
                return Ok(());
            }
            Ok(n) => n,
            Err(e) => {
                error!("Failed to read from target server {}: {}", target_addr, e);
                return Ok(());
            }
        };

        let response = String::from_utf8_lossy(&buf[..n]).to_string();
        debug!("Received response from target server: {}", response);

        // Captura o status code da resposta
        let status_line = response.lines().next().unwrap_or_default();
        let status_code = status_line.split_whitespace().nth(1).unwrap_or_default();

        // Calcula a latência
        let latency = start.elapsed().as_millis();

        // Modifica a resposta para incluir a latência no cabeçalho
        let mut response_lines = response.lines();
        let status_line = response_lines.next().ok_or("Invalid response")?;
        let mut headers = String::new();
        let mut body = String::new();
        let mut in_body = false;

        for line in response_lines {
            if in_body {
                body.push_str(line);
                body.push('\n');
            } else if line.is_empty() {
                in_body = true;
                headers.push_str(&format!("X-Balancer-Latency: {}ms\r\n", latency));
            }
            if !in_body {
                headers.push_str(line);
                headers.push('\n');
            }
        }

        let full_response = format!("{}\r\n{}\r\n{}", status_line, headers, body);

        writer.write_all(full_response.as_bytes()).await?;
        writer.flush().await?;

        if status_code.starts_with('2') {
            info!("Successfully forwarded response to client with status code {} and latency {}ms", status_code, latency);
        } else {
            warn!("Forwarded response to client with status code {} and latency {}ms", status_code, latency);
        }

        Ok(())
    }
}
