#!/bin/sh


# Wait for redis to be available
until redis-cli -h redis ping; do
  echo "Waiting for redis to be available..."
  sleep 2
done

# Set initial configuration in redis
redis-cli -h redis set gateway_config '{
  "primary_addr": "192.168.15.13:4000",
  "canary_addr": "192.168.15.13:5000",
  "canary_percentage": 50
}'

echo "Redis initialization complete"
