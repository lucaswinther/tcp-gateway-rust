# tcp-gateway-rust# TCP Gateway

Este projeto é um gateway TCP escrito em Rust que suporta roteamento baseado em configuração dinâmica armazenada no etcd.

## Requisitos

- Docker
- Docker Compose

## Configuração

A configuração é armazenada no etcd. Aqui está um exemplo de configuração JSON:

```json
{
  "primary_addr": "127.0.0.1:3001",
  "canary_addr": "127.0.0.1:3002",
  "canary_percentage": 10
}
