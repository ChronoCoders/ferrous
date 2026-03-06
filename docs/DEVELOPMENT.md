# Ferrous Network Development Guide

This guide describes how to build, test, and work on the Ferrous Network codebase.

## Prerequisites

- Rust 1.70+ (stable)
- Cargo package manager
- Git

## Building

Clone the repository and build all targets:

```bash
git clone https://github.com/chronocoders/ferrous
cd ferrous

cargo build
```

This compiles the library, tests, and the example node binary.

## Project Layout

Key directories:

- `src/`
  - `lib.rs`: Crate root, exports top-level modules.
  - `consensus/`: Blocks, transactions, UTXO set, validation, difficulty, params.
  - `mining/`: Miner implementation and PoW loop.
  - `primitives/`: Hashing, serialization, varint utilities.
  - `rpc/`: JSON-RPC server and method definitions.
  - `script/`: Script opcodes, engine, and sighash.
- `tests/`: Integration tests for consensus, mining, RPC, scripts, serialization, and networking.
- `examples/`
  - `node.rs`: Example standalone node binary exposing the RPC interface.

## Running the Node

The primary entry point is the example node:

```bash
cargo run --example node -- --network regtest --dashboard
```

Options:

- `--network <mainnet|testnet|regtest>` selects `ChainParams`.
- `--dashboard` enables the TUI dashboard for monitoring peers and blocks.
- The example binds the RPC server to `127.0.0.1:8332`.

Typical workflows:

- **Regtest**: Fast development and testing with `--network regtest` and `mineblocks` RPC calls.
- **Mainnet/Testnet**: Experimental long-running nodes. All networks currently share a single genesis block but differ in difficulty behavior.

## Testing Strategy

Ferrous uses a mix of unit tests (within modules) and integration tests (in `tests/`).

### Running Tests

```bash
# All tests
cargo test

# Specific integration suites
cargo test --test chain_tests
cargo test --test mining_tests
cargo test --test rpc_tests
cargo test --test p2p_integration

# Show output
cargo test -- --nocapture
```

### Linting

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

The crate enforces:

- `#![deny(warnings)]`
- `#![forbid(unsafe_code)]`

in [lib.rs](file:///c:/ferrous/src/lib.rs#L1-L3), so any compiler warning or unsafe usage will fail the build.

### Test Coverage

Integration suites (under `tests/`):

- `block_tests`
- `chain_tests`
- `difficulty_tests`
- `hash_tests`
- `merkle_tests`
- `mining_tests`
- `network_params_tests`
- `rpc_tests`
- `script_tests`
- `serialize_tests`
- `sighash_tests`
- `transaction_tests`
- `utxo_tests`
- `validation_tests`
- `varint_tests`
- `p2p_integration` (End-to-end networking tests)

Together with unit tests in the modules, these cover:

- Block/transaction serialization and hashing.
- Difficulty adjustment and validation.
- Chain reorganization and UTXO behavior.
- Script engine, sighash, and signature verification.
- RPC request handling and error paths.
- P2P Handshake, Block Propagation, and Peer Discovery.

Most integration tests run against the `Regtest` network parameters for speed.

## Coding Conventions

- Rust 2021 edition.
- No unsafe code.
- Prefer explicit types in public APIs.
- Use the existing module structure for new functionality (e.g. new consensus rules go under `consensus/`).
- Keep consensus-critical serialization stable; changes to binary formats must be carefully reviewed.

## Adding New Features

When adding a feature:

1. Identify the correct module:
   - Consensus changes → `consensus/`
   - Script opcodes or evaluation → `script/`
   - Mining logic → `mining/`
   - RPC additions → `rpc/`
2. Add or extend unit tests close to the implementation.
3. Add or extend integration tests under `tests/` to cover end-to-end behavior.
4. Run `cargo test` and `cargo clippy --all-targets --all-features -- -D warnings`.

For consensus changes:

- Update [docs/CONSENSUS.md](file:///c:/ferrous/docs/CONSENSUS.md) with the new rules.
- Ensure existing tests either still pass or are updated to match the new behavior.

## Troubleshooting

### Tests Failing with Locked Binaries (Windows)

On Windows, a previous test run may leave a locked test executable, causing linker errors (e.g. `LNK1104`). If you see errors mentioning a specific `*_tests.exe` file:

1. Use Task Manager or PowerShell to terminate the stuck process.
2. Re-run `cargo test`.

### JSON-RPC Issues

If RPC calls fail:

- Ensure the node is running and listening on `127.0.0.1:8332`.
- Check that the request body is valid JSON and uses the correct `method` and `params`.
- Inspect error responses for `code` and `message` fields.

### Consensus or Script Errors

When a block or transaction is rejected during testing:

- Look at the specific `ValidationError` or `ScriptError` in the test output.
- Use the existing tests in `tests/` as references for valid and invalid cases.

## Contribution Notes

Even if the project is run by a single developer, the following practices help keep the codebase healthy:

- Keep changes small and focused.
- Maintain and extend tests alongside new code.
- Update documentation in `docs/` when behavior changes.

