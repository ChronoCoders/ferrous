# Ferrous Network Development Guide

This guide describes how to build, test, and work on the Ferrous Network codebase.

## Prerequisites

- Rust 1.70+ (stable)
- Cargo package manager
- Git
- Clang (for RocksDB bindings)
- CMake

## Building

Clone the repository and build all targets:

```bash
git clone https://github.com/ChronoCoders/ferrous
cd ferrous

cargo build
```

This compiles the library and the example node binary.

Before any deploy, ensure these pass locally:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

## Build Cache (sccache)

On Windows (PowerShell), `sccache` can speed up repeated builds:

```powershell
cargo install sccache --locked
$env:RUSTC_WRAPPER = "sccache"
$env:SCCACHE_CACHE_SIZE = "2G"
```

## Project Layout

Key directories:

- `src/`
  - `lib.rs`: Crate root, exports top-level modules.
  - `consensus/`: Blocks, transactions, UTXO set, validation, difficulty, params.
  - `mining/`: Miner implementation and PoW loop.
  - `primitives/`: Hashing, serialization, varint utilities.
  - `rpc/`: JSON-RPC server and method definitions.
  - `script/`: Script opcodes, engine, and sighash.
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

- **Regtest**: Fast development and testing with `--network regtest` and `mineblocks` / `generatetoaddress` RPC calls.
- **Testnet**: Long-running nodes with P2P networking and live mining.

## Testing and Linting

```bash
cargo test
```

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

The crate enforces:

- `#![deny(warnings)]`
- `#![forbid(unsafe_code)]`

in [lib.rs](file:///c:/ferrous/src/lib.rs#L1-L3), so any compiler warning or unsafe usage will fail the build.

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
2. Add or extend tests close to the implementation.
3. Run `cargo test` and `cargo clippy --all-targets --all-features -- -D warnings`.

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

## Contribution Notes

Even if the project is run by a single developer, the following practices help keep the codebase healthy:

- Keep changes small and focused.
- Maintain and extend tests alongside new code.
- Update documentation in `docs/` when behavior changes.
