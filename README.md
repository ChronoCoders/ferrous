# Ferrous Network

A Bitcoin-inspired blockchain implementation in Rust, featuring Proof-of-Work consensus, UTXO transactions, and a stack-based scripting engine.

## Features

- **Nakamoto Consensus**: Proof-of-Work with SHA256d hashing
- **UTXO Model**: Unspent Transaction Output tracking with witness support
- **Script Engine**: Stack-based VM supporting P2PKH and P2WPKH
- **Difficulty Adjustment**: Per-block target retargeting with timespan clamped to \[1/4, 4\] × target block time
- **Chain Reorganization**: Automatic reorg handling with cumulative work comparison
- **Network Modes**: Mainnet, Testnet, and Regtest configurations via `ChainParams`
- **JSON-RPC Interface**: HTTP-based node control and queries
- **Zero Unsafe Code**: Memory-safe implementation with comprehensive testing

## Quick Start

### Prerequisites
- Rust 1.70+ (stable)
- Cargo package manager

### Build & Run

```bash
# Clone repository
git clone https://github.com/yourusername/ferrous
cd ferrous

# Run tests
cargo test

# Start node (Regtest mode - fast mining)
cargo run --example node -- --network regtest

# Start node (Mainnet mode)
cargo run --example node -- --network mainnet
```

### Mine Blocks via RPC

```powershell
# PowerShell
$body = @{
    jsonrpc = "2.0"
    method = "mineblocks"
    params = @(10)
    id = 1
} | ConvertTo-Json

Invoke-RestMethod -Uri http://127.0.0.1:8332 -Method Post -Body $body -ContentType "application/json"
```

```bash
# Bash
curl -X POST http://127.0.0.1:8332 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"mineblocks","params":[10],"id":1}'
```

## Project Status

**Current Phase**: Single-node testnet  
**Version**: 0.1.0  
**Completion**: ~60% of full node implementation

### Implemented ✅

- Complete consensus engine (PoW, difficulty adjustment, chain selection)
- UTXO state management with coinbase maturity and basic double-spend prevention
- Block validation (Merkle roots, weight, structure)
- Mining with per-block difficulty adjustment
- Script execution (P2PKH, P2WPKH)
- Signature verification (ECDSA over secp256k1)
- JSON-RPC server (`getblockchaininfo`, `mineblocks`, `getblock`, `getbestblockhash`, `stop`)
- Network parameter separation (Mainnet/Testnet/Regtest via `ChainParams`)

### In Progress 🚧

- Block/UTXO persistence (embedded key-value store)
- P2P networking layer
- Transaction mempool

### Planned 📋

- Wallet functionality
- Full node sync
- Multi-signature support
- Additional script opcodes

## Documentation

- [Architecture Overview](docs/ARCHITECTURE.md)
- [Consensus Specification](docs/CONSENSUS.md)
- [RPC API Reference](docs/API.md)
- [Development Guide](docs/DEVELOPMENT.md)

## Testing

```bash
# Run all tests (unit + integration)
cargo test

# Run specific integration suites
cargo test --test chain_tests
cargo test --test mining_tests
cargo test --test rpc_tests

# Show test output
cargo test -- --nocapture

# Clippy lints
cargo clippy --all-targets --all-features -- -D warnings
```

**Test Statistics**:
- Integration suites: 15 test binaries under `tests/`
- Unit tests: focused on consensus, mining, script engine
- Coverage: Core consensus, mining, RPC, scripts, serialization, UTXO
- Regtest is used in most integration tests for fast execution

## Performance

**Regtest Mode** (Development):
- Block time: Fast (easy PoW target, no difficulty retargeting)
- Difficulty: Constant
- Use case: Rapid testing, development

**Testnet Mode**:
- Target block time: ~150 seconds
- Difficulty: Adjusts per block based on previous timestamp
- Use case: Public testing, experimentation

**Mainnet Mode**:
- Target block time: 150 seconds
- Difficulty: Per-block adjustment with clamped timespan
- Use case: Production (when ready)

## License

[Your License Here]

## Contact

[Your Contact Info]

