use rand::Rng;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crate::config::ConfigManager;
use tracing::{info, error, debug, warn};
use tokio::time::{timeout, Instant};
use prometheus::{Encoder, TextEncoder, IntCounterVec, IntGauge, HistogramVec, Opts, Registry};
use std::net::ToSocketAddrs;
use std::sync::Arc;
use std::time::Duration;

pub struct Balancer {
    config_manager: ConfigManager,
    registry: Arc<Registry>,
    request_counter: IntCounterVec,
    response_counter: IntCounterVec,
    latency_histogram: HistogramVec,
    primary_status: IntGauge,
    canary_status: IntGauge,
    server_response_counter: IntCounterVec,
}

impl Balancer {
    // Cria uma nova instância do balanceador de carga
    pub fn new(config_manager: ConfigManager) -> Self {
        let registry = Arc::new(Registry::new());

        let request_counter = IntCounterVec::new(
            Opts::new("requests_total", "Total number of requests received"),
            &["endpoint"],
        ).unwrap();
        let response_counter = IntCounterVec::new(
            Opts::new("responses_total", "Total number of responses sent"),
            &["endpoint", "status_code"],
        ).unwrap();
        let latency_histogram = HistogramVec::new(
            Opts::new("request_latency_seconds", "Request latency in seconds").into(),
            &["endpoint"],
        ).unwrap();
        let primary_status = IntGauge::new("primary_status", "Status of primary server").unwrap();
        let canary_status = IntGauge::new("canary_status", "Status of canary server").unwrap();
        let server_response_counter = IntCounterVec::new(
            Opts::new("server_responses_total", "Total number of responses by server"),
            &["server", "status_code"],
        ).unwrap();

        registry.register(Box::new(request_counter.clone())).unwrap();
        registry.register(Box::new(response_counter.clone())).unwrap();
        registry.register(Box::new(latency_histogram.clone())).unwrap();
        registry.register(Box::new(primary_status.clone())).unwrap();
        registry.register(Box::new(canary_status.clone())).unwrap();
        registry.register(Box::new(server_response_counter.clone())).unwrap();

        Balancer {
            config_manager,
            registry,
            request_counter,
            response_counter,
            latency_histogram,
            primary_status,
            canary_status,
            server_response_counter,
        }
    }

    // Inicia o balanceador de carga e escuta por conexões TCP
    pub async fn run(&self, listen_addr: &str) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(listen_addr).await?;
        info!("Balancer running on {}", listen_addr);

        let metrics_registry = self.registry.clone();

        // Inicia o servidor de métricas
        let metrics_listener = TcpListener::bind("0.0.0.0:8081").await?;
        tokio::spawn(async move {
            if let Err(e) = Self::run_metrics_server(metrics_listener, metrics_registry).await {
                error!("Metrics server error: {}", e);
            }
        });

        loop {
            let (socket, addr) = listener.accept().await?;
            debug!("Accepted connection from {}", addr);

            let config_manager = self.config_manager.clone();
            let request_counter = self.request_counter.clone();
            let response_counter = self.response_counter.clone();
            let latency_histogram = self.latency_histogram.clone();
            let primary_status = self.primary_status.clone();
            let canary_status = self.canary_status.clone();
            let server_response_counter = self.server_response_counter.clone();

            tokio::spawn(async move {
                if let Err(e) = Balancer::handle_connection(
                    socket,
                    config_manager,
                    request_counter,
                    response_counter,
                    latency_histogram,
                    primary_status,
                    canary_status,
                    server_response_counter,
                ).await {
                    error!("Error handling connection: {}", e);
                }
            });
        }
    }

    // Manipula uma conexão recebida, encaminhando a requisição para o servidor de destino e retornando a resposta
    async fn handle_connection(
        socket: TcpStream,
        config_manager: ConfigManager,
        request_counter: IntCounterVec,
        response_counter: IntCounterVec,
        latency_histogram: HistogramVec,
        primary_status: IntGauge,
        canary_status: IntGauge,
        server_response_counter: IntCounterVec,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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

        // Incrementa o contador de requisições
        request_counter.with_label_values(&["gateway"]).inc();

        // Obtém a configuração atual e o status dos servidores
        let config = config_manager.get_config().await;
        let server_status = config_manager.get_server_status().await;

        primary_status.set(if server_status.primary_online { 1 } else { 0 });
        canary_status.set(if server_status.canary_online { 1 } else { 0 });

        let (target_addr, fallback_addr, server_label, fallback_label) = if server_status.primary_online && server_status.canary_online {
            // Se ambos os servidores estiverem online, use a porcentagem configurada para decidir
            if rand::thread_rng().gen_range(0..100) < config.canary_percentage {
                (&config.canary_addr, &config.primary_addr, "canary", "primary")
            } else {
                (&config.primary_addr, &config.canary_addr, "primary", "canary")
            }
        } else if server_status.primary_online {
            (&config.primary_addr, &config.canary_addr, "primary", "canary")
        } else if server_status.canary_online {
            (&config.canary_addr, &config.primary_addr, "canary", "primary")
        } else {
            // Se nenhum servidor estiver online, retorne uma resposta de erro
            let error_message = "HTTP/1.1 503 Service Unavailable\r\n\r\nNo available servers";
            writer.write_all(error_message.as_bytes()).await?;
            writer.flush().await?;
            error!("No available servers. Sent 503 Service Unavailable to client.");
            response_counter.with_label_values(&["gateway", "503"]).inc();
            return Ok(());
        };

        // Tenta a conexão com o servidor de destino
        let result = Self::forward_request(
            target_addr,
            &request,
            &mut writer,
            server_label,
            &response_counter,
            &server_response_counter,
            &latency_histogram,
            start,
        ).await;

        // Se a conexão foi recusada, tenta se conectar ao servidor de fallback
        if let Err(e) = result {
            if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
                if io_err.kind() == std::io::ErrorKind::ConnectionRefused {
                    warn!("Retrying request to fallback server: {}", fallback_addr);
                    Self::forward_request(
                        fallback_addr,
                        &request,
                        &mut writer,
                        fallback_label,
                        &response_counter,
                        &server_response_counter,
                        &latency_histogram,
                        start,
                    ).await?;
                } else {
                    return Err(e);
                }
            } else {
                return Err(e);
            }
        }

        Ok(())
    }

    // Encaminha a requisição para o servidor de destino
    async fn forward_request(
        target_addr: &str,
        request: &str,
        writer: &mut tokio::net::tcp::OwnedWriteHalf,
        server_label: &str,
        response_counter: &IntCounterVec,
        server_response_counter: &IntCounterVec,
        latency_histogram: &HistogramVec,
        start: Instant,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let timeout_duration = Duration::from_secs(5); // Timeout de 5 segundos para todas as operações
        let target_socket = match timeout(timeout_duration, TcpStream::connect(target_addr.to_socket_addrs()?.next().unwrap())).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(e)) => {
                error!("Failed to connect to target server {}: {}", target_addr, e);
                let error_message = "HTTP/1.1 502 Bad Gateway\r\n\r\nFailed to connect to target server";
                writer.write_all(error_message.as_bytes()).await?;
                writer.flush().await?;
                response_counter.with_label_values(&["gateway", "502"]).inc();
                server_response_counter.with_label_values(&[server_label, "502"]).inc();
                return Err(Box::new(e));
            }
            Err(_) => {
                error!("Timeout while connecting to target server {}", target_addr);
                let error_message = "HTTP/1.1 504 Gateway Timeout\r\n\r\nTimeout while connecting to target server";
                writer.write_all(error_message.as_bytes()).await?;
                writer.flush().await?;
                response_counter.with_label_values(&["gateway", "504"]).inc();
                server_response_counter.with_label_values(&[server_label, "504"]).inc();
                return Err("Timeout while connecting to target server".into());
            }
        };

        let (mut target_reader, mut target_writer) = target_socket.into_split();
        timeout(timeout_duration, target_writer.write_all(request.as_bytes())).await??;
        timeout(timeout_duration, target_writer.flush()).await??;

        let mut buf = vec![0; 4096];
        let n = match timeout(timeout_duration, target_reader.read(&mut buf)).await {
            Ok(Ok(n)) if n == 0 => {
                warn!("Target server {} closed the connection", target_addr);
                return Ok(());
            }
            Ok(Ok(n)) => n,
            Ok(Err(e)) => {
                error!("Failed to read from target server {}: {}", target_addr, e);
                return Err(Box::new(e));
            }
            Err(_) => {
                error!("Timeout while reading from target server {}", target_addr);
                return Err("Timeout while reading from target server".into());
            }
        };

        let response = String::from_utf8_lossy(&buf[..n]).to_string();
        debug!("Received response from target server: {}", response);

        // Captura o status code da resposta
        let status_line = response.lines().next().unwrap_or_default();
        let status_code = status_line.split_whitespace().nth(1).unwrap_or_default();

        // Calcula a latência
        let latency = start.elapsed().as_secs_f64();
        latency_histogram.with_label_values(&["gateway"]).observe(latency);

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
                headers.push_str(&format!("X-Balancer-Latency: {}ms\r\n", latency * 1000.0));
            }
            if !in_body {
                headers.push_str(line);
                headers.push('\n');
            }
        }

        let full_response = format!("{}\r\n{}\r\n{}", status_line, headers, body);

        timeout(timeout_duration, writer.write_all(full_response.as_bytes())).await??;
        timeout(timeout_duration, writer.flush()).await??;

        response_counter.with_label_values(&["gateway", status_code]).inc();
        server_response_counter.with_label_values(&[server_label, status_code]).inc();

        if status_code.starts_with('2') {
            debug!("Successfully forwarded response to client with status code {} and latency {}ms", status_code, latency * 1000.0);
        } else {
            warn!("Forwarded response to client with status code {} and latency {}ms", status_code, latency * 1000.0);
        }

        Ok(())
    }

    // Servidor de métricas
    async fn run_metrics_server(listener: TcpListener, registry: Arc<Registry>) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            let (mut socket, _) = listener.accept().await?;
            let mut buffer = [0; 1024];
            let _ = socket.read(&mut buffer).await?;

            let encoder = TextEncoder::new();
            let metric_families = registry.gather();
            let mut metrics = Vec::new();
            encoder.encode(&metric_families, &mut metrics).unwrap();

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\n\r\n{}",
                encoder.format_type(),
                metrics.len(),
                String::from_utf8(metrics).unwrap()
            );

            socket.write_all(response.as_bytes()).await?;
            socket.flush().await?;
        }
    }
}
