# Ferrous Network

A next-generation, memory-safe Layer 1 blockchain engineered from the ground up in Rust. Ferrous combines the battle-tested security of Bitcoin-like Proof-of-Work consensus with a modern, modular architecture designed for high performance and long-term extensibility.

Featuring a custom-built, asynchronous P2P networking stack with automatic partition recovery, persistent RocksDB storage, and a strict "zero-warnings" code quality policy, Ferrous serves as both a production-ready foundation for decentralized applications and a reference implementation for future cryptographic upgrades (Post-Quantum signatures, RingCT privacy).

## Features

- **Consensus**: SHA256d Proof-of-Work with per-block difficulty adjustment (150s target).
- **Networking**: Full P2P stack with headers-first sync, block relay, and inventory protocol.
- **Storage**: Persistent blockchain state using RocksDB.
- **Architecture**: Modular design separating consensus, networking, and storage logic.
- **Interface**: JSON-RPC API and TUI Dashboard for node monitoring.
- **Safety**: 100% safe Rust code with strict linting (`deny(warnings)`).

## Quick Start

### Prerequisites
- Rust 1.70+ (stable)
- Clang (for RocksDB bindings)
- CMake

### Build & Run

```bash
# Clone repository
git clone https://github.com/ChronoCoders/ferrous
cd ferrous

# Run the full node with TUI dashboard (Regtest mode)
cargo run --example node -- --dashboard --network regtest

# Run on Mainnet
cargo run --example node -- --network mainnet
```

### Monitor TUI (Seed Nodes)

Ferrous includes a standalone monitoring TUI that polls multiple remote nodes over JSON-RPC via SSH tunnels.

1) Open SSH tunnels (these commands will appear to “do nothing” and should be left running):

```bash
ssh -N -L 18331:127.0.0.1:8332 root@45.77.153.141
ssh -N -L 18332:127.0.0.1:8332 root@45.77.64.221
```

2) Verify tunnels (PowerShell):

```powershell
curl.exe -s -X POST http://127.0.0.1:18331 -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","method":"getblockchaininfo","params":[],"id":1}'
curl.exe -s -X POST http://127.0.0.1:18332 -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","method":"getblockchaininfo","params":[],"id":1}'
```

3) Run the monitor:

```bash
cargo run --release --example monitor
```

### Mining (Regtest)

To mine blocks instantly in `regtest` mode, open a second terminal and use the RPC interface:

```powershell
# PowerShell
Invoke-RestMethod -Uri http://127.0.0.1:8332 -Method Post -Body '{"jsonrpc": "2.0", "method": "mineblocks", "params": [10], "id": 1}' -ContentType "application/json"
```

```bash
# Bash
curl -X POST http://127.0.0.1:8332 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"mineblocks","params":[10],"id":1}'
```

## Project Status

**Version**: 0.1.0 (Alpha)
**Phase**: Phase 1 Foundation (In Progress)

### Implemented

- **Core**: Block/Tx validation, Merkle roots, UTXO set management.
- **Networking**:
  - Handshake (Version/Verack)
  - Headers-first synchronization
  - Block propagation (Inv/GetData/Block)
  - Transaction relay & Mempool
  - Peer discovery (Addr/GetAddr)
  - **Network Recovery**:
    - Automatic partition detection and reconnection.
    - Health monitoring and stale block detection.
    - RPC commands: `getrecoverystatus`, `forcereconnect`, `resetnetwork`.
- **Storage**: RocksDB integration for chain state and block index.
- **RPC**: JSON-RPC API with the following methods:
  - `getblockchaininfo`
  - `getblockhash`
  - `getblock`
  - `getbestblockhash`
  - `getmininginfo`
  - `mineblocks`
  - `generatetoaddress`
  - `getnewaddress`
  - `getbalance`
  - `listunspent`
  - `listaddresses`
  - `sendtoaddress`
  - `getnetworkinfo`
  - `getpeerinfo`
  - `getconnectioncount`
  - `resetnetwork`
  - `stop`
- **UI**: Terminal User Interface (TUI) for real-time statistics.

### Infrastructure (Live Testnet)

- `seed1.ferrous.network` — `45.77.153.141` — Vultr New York — mining
- `seed4.ferrous.network` — `45.77.64.221` — Vultr Frankfurt — mining

### Roadmap

- **Phase 1 (Current)**: Bitcoin-like foundation — PoW, UTXO, P2P, headers-first IBD (in progress: parallel IBD Phase 1 deployed).
- **Phase 2**: Parallel IBD completion (Phase 2 + Phase 3) + testnet reset.
- **Phase 3**: Wallet integration — BIP39 seed phrase + Shamir's Secret Sharing recovery.
- **Phase 4**: Post-Quantum Cryptography — CRYSTALS-Dilithium (before Ring CT).
- **Phase 5**: Privacy Features — Ring Confidential Transactions + CLSAG.
- **Phase 6**: Security audit + Mainnet launch.

## Documentation

- [Architecture Overview](docs/ARCHITECTURE.md)
- [Consensus Specification](docs/CONSENSUS.md)
- [API Reference](docs/API.md)
- [Development Guide](docs/DEVELOPMENT.md)

## Testing

The project maintains a strict "zero warnings" policy.

```bash
# Run tests
cargo test

# Run strict linting checks
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

## License

MIT License. See [LICENSE](LICENSE) for details.

## Contact

Maintained by **ChronoCoders**.
