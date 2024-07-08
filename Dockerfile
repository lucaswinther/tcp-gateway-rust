# Use uma imagem oficial Rust como a base
FROM rust:1.78 as builder

# Crie um diretório de trabalho
WORKDIR /app

# Copie os arquivos Cargo.toml e Cargo.lock
COPY Cargo.toml Cargo.lock ./

# Crie uma diretório src vazio para a compilação do Cargo
RUN mkdir src

# Copie os arquivos de origem do projeto
COPY src ./src

# Compile o projeto para a release
RUN cargo build --release

# Use uma imagem menor para a execução
FROM debian:bookworm-slim

# Crie um diretório para a aplicação
WORKDIR /app

# Copie o binário da fase de compilação
COPY --from=builder /app/target/release/tcp-gateway-rust .

EXPOSE 8080
EXPOSE 8081

# Execute a aplicação
CMD ["./tcp-gateway-rust"]
