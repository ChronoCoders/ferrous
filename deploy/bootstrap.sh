#!/bin/bash
set -e

# Ferrous Network Bootstrap Script
# Run this ONCE on seed1 to initialize the network

echo "=== Ferrous Network Bootstrap ==="
echo "This will create the genesis block and start the network"
echo ""

# Configuration
DATA_DIR="/var/lib/ferrous"
GENESIS_ADDR="tfer1qxqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqgqqqq"

# Stop service if running
sudo systemctl stop ferrous || true

# Clear any existing data
echo "Clearing existing data..."
sudo rm -rf $DATA_DIR/*

# Start node temporarily
echo "Starting bootstrap node..."
sudo -u ferrous /usr/local/bin/ferrous-node \
    --config /etc/ferrous/config.toml \
    --data-dir $DATA_DIR &

NODE_PID=$!
sleep 5

# Create genesis via RPC
echo "Creating genesis block..."
curl -s -u ferrous:${RPC_PASSWORD} \
    --data-binary '{"jsonrpc":"2.0","id":"bootstrap","method":"mineblocks","params":[1]}' \
    -H 'content-type: text/plain;' \
    http://127.0.0.1:8332/

# Mine initial blocks (10 for maturity testing)
echo "Mining initial blocks..."
curl -s -u ferrous:${RPC_PASSWORD} \
    --data-binary '{"jsonrpc":"2.0","id":"bootstrap","method":"mineblocks","params":[10]}' \
    -H 'content-type: text/plain;' \
    http://127.0.0.1:8332/

# Stop temporary node
kill $NODE_PID
sleep 2

# Start as service
echo "Starting production service..."
sudo systemctl start ferrous

echo ""
echo "=== Bootstrap Complete ==="
echo "Genesis hash: 48ddae95500e21bff39f823ebabfd65df7fa1b796bb867f0351bc7bf773c1c43"
echo "Initial blocks: 11"
echo ""
echo "Other nodes will sync from this node automatically"
