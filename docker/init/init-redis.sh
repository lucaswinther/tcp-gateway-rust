#!/bin/sh

# Wait for redis to be available
until redis-cli -h redis ping; do
  echo "Waiting for redis to be available..."
  sleep 2
done

# Set initial configuration in redis
redis-cli -h redis set gateway_config '{
  "primary_addr": "127.0.0.1:4000",
  "canary_addr": "127.0.0.1:5000",
  "canary_percentage": 10
}'

echo "Redis initialization complete"
