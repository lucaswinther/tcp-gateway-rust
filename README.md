# TCP Gateway Load Balancer

This project is a TCP load balancer written in Rust. It forwards incoming connections to backend servers based on their health status and a canary percentage.

## Features
- Load balancing between primary and canary servers.
- Health checks for backend servers.
- Prometheus metrics for monitoring.
- Configuration management via Redis.
- Graceful handling of errors and retries.

## Configuration
Environment variables:
- `REDIS_URL`: Redis connection URL.
- `CONFIG_UPDATE_INTERVAL`: Interval in seconds to update configuration from Redis.
- `STATUS_UPDATE_INTERVAL`: Interval in seconds to check server statuses.

## Running the Project
1. Ensure Redis is running and accessible.
2. Create a `.env` file with the necessary environment variables.
3. Build and run the project using Cargo:
    ```sh
    cargo build
    cargo run
    ```

## Dependencies
- `tokio`: Asynchronous runtime.
- `serde`: Serialization and deserialization.
- `rand`: Random number generation.
- `redis`: Redis client.
- `dotenv`: Environment variable management.
- `tracing`: Logging and diagnostics.
- `prometheus`: Metrics collection and exposition.

## Metrics
Prometheus metrics are exposed on port `8081`. You can scrape these metrics using a Prometheus server.
