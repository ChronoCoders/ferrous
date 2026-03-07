# Ferrous Network Testnet Deployment

## Seed Node Infrastructure

**Global Distribution:**
- seed1: New York (US East)
- seed2: Atlanta (US Southeast) 
- seed3: San Francisco (US West)
- seed4: Frankfurt (EU Central)
- seed5: Singapore (Asia Pacific)

**Network:** testnet
**Port:** 8333
**RPC Port:** 8332 (localhost only)

## Deployment Steps

### 1. Build Release Binary
```bash
cargo build --release --bin ferrous-node
```

### 2. Deploy to Each Node

On each seed node:
```bash
# Copy binary and config
scp target/release/ferrous-node root@seedN.ferrous.network:/tmp/
scp config/testnet.toml root@seedN.ferrous.network:/tmp/
scp deploy/*.sh root@seedN.ferrous.network:/tmp/

# SSH and configure
ssh root@seedN.ferrous.network
cd /tmp
chmod +x *.sh
./node_config.sh seedN
```

### 3. Bootstrap Network (seed1 ONLY)

On seed1.ferrous.network:
```bash
export RPC_PASSWORD="your-secure-password"
./bootstrap.sh
```

### 4. Start Other Nodes

On seed2-5:
```bash
sudo systemctl start ferrous
```

Nodes will automatically sync from seed1.

### 5. Verify Network

On each node:
```bash
./health_check.sh
```

Expected output:
- Status: HEALTHY
- Block Height: 11 (initially)
- Peer Connections: 4

## Monitoring

### Service Status
```bash
sudo systemctl status ferrous
```

### Live Logs
```bash
sudo journalctl -u ferrous -f
```

### RPC Query
```bash
curl -u ferrous:password \
  --data-binary '{"jsonrpc":"2.0","id":"1","method":"getblockchaininfo","params":[]}' \
  http://127.0.0.1:8332/
```

## Firewall Configuration

Open port 8333:
```bash
sudo ufw allow 8333/tcp
sudo ufw enable
```

## Security

- RPC bound to localhost only
- Strong RPC password via environment variable
- Service runs as dedicated `ferrous` user
- Data directory: `/var/lib/ferrous`
- Logs: `/var/log/ferrous/`

## Troubleshooting

### Node won't start
```bash
sudo journalctl -u ferrous -n 50
```

### No peer connections
- Check firewall: `sudo ufw status`
- Check DNS: `dig seed1.ferrous.network`
- Check connectivity: `telnet seed1.ferrous.network 8333`

### Sync issues
- Verify genesis hash matches
- Check logs for validation errors
- Restart with: `sudo systemctl restart ferrous`
