#!/bin/bash
set -e

# Ferrous Network Node Configuration Script

NODE_NAME=${1:-"seed1"}
DATA_DIR="/var/lib/ferrous"
LOG_DIR="/var/log/ferrous"
BIN_DIR="/usr/local/bin"

echo "Configuring Ferrous node: $NODE_NAME"

# Create directories
sudo mkdir -p $DATA_DIR
sudo mkdir -p $LOG_DIR
sudo mkdir -p /etc/ferrous

# Install binary
sudo cp target/release/ferrous-node $BIN_DIR/ferrous-node
sudo chmod +x $BIN_DIR/ferrous-node

# Copy configuration
sudo cp config/testnet.toml /etc/ferrous/config.toml

# Create systemd service
cat <<EOF | sudo tee /etc/systemd/system/ferrous.service
[Unit]
Description=Ferrous Network Node
After=network.target

[Service]
Type=simple
User=ferrous
Group=ferrous
WorkingDirectory=$DATA_DIR
ExecStart=$BIN_DIR/ferrous-node --config /etc/ferrous/config.toml
Restart=always
RestartSec=10

# Resource limits
LimitNOFILE=65536
LimitNPROC=4096

# Logging
StandardOutput=append:$LOG_DIR/node.log
StandardError=append:$LOG_DIR/error.log

[Install]
WantedBy=multi-user.target
EOF

# Create ferrous user
sudo useradd -r -s /bin/false ferrous || true
sudo chown -R ferrous:ferrous $DATA_DIR
sudo chown -R ferrous:ferrous $LOG_DIR

# Enable and start service
sudo systemctl daemon-reload
sudo systemctl enable ferrous
sudo systemctl start ferrous

echo "Node configured successfully"
echo "Status: sudo systemctl status ferrous"
echo "Logs: sudo journalctl -u ferrous -f"
