# Ferrous Network

A high-performance, memory-safe Layer 1 blockchain implementation in Rust, featuring Proof-of-Work consensus, a complete P2P networking stack, and a modular architecture designed for future post-quantum upgrades.

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)
![Build Status](https://img.shields.io/badge/build-passing-brightgreen.svg)

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

**Version**: 0.2.0 (Alpha)  
**Phase**: P2P Network Implementation (Complete)

### Implemented ✅

- **Core**: Block/Tx validation, Merkle roots, UTXO set management.
- **Networking**:
  - Handshake (Version/Verack)
  - Headers-first synchronization
  - Block propagation (Inv/GetData/Block)
  - Transaction relay & Mempool
  - Peer discovery (Addr/GetAddr)
- **Storage**: RocksDB integration for chain state and block index.
- **RPC**: Full suite of control commands (`getblockchaininfo`, `mineblocks`, `getpeerinfo`, etc.).
- **UI**: Terminal User Interface (TUI) for real-time statistics.

### Roadmap 🗺️

- **Phase 1 (Current)**: Bitcoin-like foundation (PoW, UTXO, P2P).
- **Phase 2**: Wallet integration and transaction management.
- **Phase 3**: Post-Quantum Cryptography (CRYSTALS-Dilithium signatures).
- **Phase 4**: Privacy Features (Ring Confidential Transactions).

## Documentation

- [Architecture Overview](docs/ARCHITECTURE.md)
- [Consensus Specification](docs/CONSENSUS.md)
- [API Reference](docs/API.md)
- [Development Guide](docs/DEVELOPMENT.md)

## Testing

The project maintains a strict "zero warnings" policy.

```bash
# Run unit and integration tests
cargo test

# Run specific P2P integration tests
cargo test --test p2p_integration

# Run strict linting checks
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

## License

MIT License. See [LICENSE](LICENSE) for details.

## Contact

Maintained by **ChronoCoders**.
