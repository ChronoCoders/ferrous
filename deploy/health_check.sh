#!/bin/bash

# Ferrous Network Health Check Script

RPC_URL="http://127.0.0.1:8332"
RPC_USER="ferrous"
RPC_PASS="${RPC_PASSWORD}"

# Check if service is running
if ! systemctl is-active --quiet ferrous; then
    echo "ERROR: Ferrous service not running"
    exit 1
fi

# Check RPC connectivity
RESPONSE=$(curl -s -u $RPC_USER:$RPC_PASS \
    --data-binary '{"jsonrpc":"2.0","id":"health","method":"getblockchaininfo","params":[]}' \
    -H 'content-type: text/plain;' \
    $RPC_URL)

if [ $? -ne 0 ]; then
    echo "ERROR: RPC not responding"
    exit 1
fi

# Parse blockchain info
HEIGHT=$(echo $RESPONSE | jq -r '.result.height')
PEERS=$(echo $RESPONSE | jq -r '.result.connections')

echo "=== Ferrous Node Health ==="
echo "Status: HEALTHY"
echo "Block Height: $HEIGHT"
echo "Peer Connections: $PEERS"
echo "Timestamp: $(date)"

# Check peer count
if [ "$PEERS" -lt 2 ]; then
    echo "WARNING: Low peer count (expected 4+)"
fi

# Check disk space
DISK_USAGE=$(df -h /var/lib/ferrous | tail -1 | awk '{print $5}' | sed 's/%//')
if [ "$DISK_USAGE" -gt 80 ]; then
    echo "WARNING: Disk usage high: ${DISK_USAGE}%"
fi

echo "=========================="
exit 0
