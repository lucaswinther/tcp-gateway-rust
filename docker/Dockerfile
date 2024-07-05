# Etapa de construção
FROM rust:1.59 as builder

WORKDIR /app

# Copie o arquivo de dependências e crie uma camada de cache
COPY Cargo.toml Cargo.lock ./
RUN mkdir src
RUN echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# Copie o código-fonte e compile
COPY . .
RUN cargo build --release

# Etapa de execução
FROM debian:buster-slim

# Instale as dependências necessárias para executar a aplicação Rust
RUN apt-get update && apt-get install -y libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copie o binário compilado da etapa de construção
COPY --from=builder /app/target/release/tcp_gateway /usr/local/bin/tcp_gateway

# Exponha a porta que o serviço irá rodar
EXPOSE 3000
EXPOSE 9090

# Comando para rodar a aplicação
CMD ["tcp_gateway"]
