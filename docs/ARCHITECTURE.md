# Ferrous Network Architecture

This document describes the internal architecture of the Ferrous Network node, including modules, data flow, key abstractions, and the concurrency model.

## Module Overview

The crate root is defined in [lib.rs](file:///c:/ferrous/src/lib.rs#L1-L8) and exposes five top-level modules:

- [consensus](file:///c:/ferrous/src/consensus/mod.rs)
- [mining](file:///c:/ferrous/src/mining/mod.rs)
- [primitives](file:///c:/ferrous/src/primitives/mod.rs)
- [rpc](file:///c:/ferrous/src/rpc/mod.rs)
- [script](file:///c:/ferrous/src/script/mod.rs)

### consensus

Core blockchain logic:

- [block.rs](file:///c:/ferrous/src/consensus/block.rs): `BlockHeader`, `U256`, target decoding and PoW checks.
- [transaction.rs](file:///c:/ferrous/src/consensus/transaction.rs): `Transaction`, `TxInput`, `TxOutput`, `Witness`, serialization and basic structure checks.
- [utxo.rs](file:///c:/ferrous/src/consensus/utxo.rs): In-memory UTXO set (`UtxoSet`) and spend/apply logic.
- [chain.rs](file:///c:/ferrous/src/consensus/chain.rs): `ChainState`, block storage, reorganization, and MedianTimePast.
- [difficulty.rs](file:///c:/ferrous/src/consensus/difficulty.rs): Difficulty target calculation and validation.
- [validation.rs](file:///c:/ferrous/src/consensus/validation.rs): Block-level consensus rules (weight, coinbase, timestamps, witness commitments).
- [merkle.rs](file:///c:/ferrous/src/consensus/merkle.rs): Merkle and witness merkle tree construction.
- [params.rs](file:///c:/ferrous/src/consensus/params.rs): `ChainParams` and `Network` (Mainnet/Testnet/Regtest).

### mining

Block template construction and PoW search:

- [mining/mod.rs](file:///c:/ferrous/src/mining/mod.rs): Module glue.
- [miner.rs](file:///c:/ferrous/src/mining/miner.rs): `Miner` type, coinbase construction, timestamp selection, and nonce search.

### script

Stack-based script engine:

- [opcodes.rs](file:///c:/ferrous/src/script/opcodes.rs): Opcode enumeration and decoding.
- [engine.rs](file:///c:/ferrous/src/script/engine.rs): Interpreter, P2PKH/P2WPKH validation, signature checks.
- [sighash.rs](file:///c:/ferrous/src/script/sighash.rs): Transaction digest computation for signatures.

### rpc

JSON-RPC interface:

- [server.rs](file:///c:/ferrous/src/rpc/server.rs): HTTP server, request handling, and method dispatch.
- [methods.rs](file:///c:/ferrous/src/rpc/methods.rs): Request/response structs for RPC methods.

### primitives

Low-level utilities:

- [hash.rs](file:///c:/ferrous/src/primitives/hash.rs): `sha256d` and `Hash256` type alias.
- [serialize.rs](file:///c:/ferrous/src/primitives/serialize.rs): Custom binary encoding/decoding traits.
- [varint.rs](file:///c:/ferrous/src/primitives/varint.rs): Bitcoin-style VarInt encoding.

## Key Abstractions

### ChainState

Defined in [chain.rs](file:///c:/ferrous/src/consensus/chain.rs#L28-L35), `ChainState` holds the in-memory view of the active chain:

- Map of block hash to `BlockData` (header, transactions, height, cumulative work).
- Height index (height → block hash).
- Tip hash and height.
- `UtxoSet` for validating new transactions.
- `ChainParams` for the selected network.

Key responsibilities:

- `new(genesis, genesis_tx, params)` initializes the chain with a single genesis block and UTXO state.
- `add_block(header, transactions)` validates difficulty, structure, and UTXO rules, then updates chain state and triggers reorgs.
- `reorganize(new_tip)` rebuilds the UTXO set along the new best chain using cumulative work.
- `median_time_past()` computes MTP over up to 11 previous blocks for timestamp rules and mining.

### UtxoSet

Defined in [utxo.rs](file:///c:/ferrous/src/consensus/utxo.rs), `UtxoSet` is an in-memory `HashMap` keyed by outpoints. It:

- Tracks spendable outputs for all blocks on the active chain.
- Enforces coinbase maturity (via `COINBASE_MATURITY` in [validation.rs](file:///c:/ferrous/src/consensus/validation.rs#L8-L10)).
- Prevents double spends when applying new transactions.

### Miner

Defined in [miner.rs](file:///c:/ferrous/src/mining/miner.rs#L10-L14), `Miner` encapsulates:

- Network parameters (`ChainParams`) to compute the current target.
- Mining address (scriptPubKey bytes) to receive coinbase rewards.

Responsibilities:

- `create_coinbase(height, subsidy, fees)` builds a coinbase transaction that commits to the block height in `script_sig`.
- `mine_block(chain, transactions)` constructs a block template, computes the target via `calculate_next_target`, and searches nonces until `check_proof_of_work` passes.
- `mine_and_attach(chain, transactions)` mines a block and attaches it to `ChainState` in one operation.

### RpcServer

Defined in [server.rs](file:///c:/ferrous/src/rpc/server.rs#L8-L12), `RpcServer` owns:

- `Arc<Mutex<ChainState>>` for synchronized access to the chain.
- `Arc<Miner>` for mining operations.
- A `tiny_http::Server` instance bound to the configured address.

Responsibilities:

- `run()` accepts HTTP requests and dispatches JSON-RPC calls.
- Implements `getblockchaininfo`, `mineblocks`, `getblock`, `getbestblockhash`, and `stop`.

## Data Flow

### Mining and Block Application

1. The node is started from [examples/node.rs](file:///c:/ferrous/examples/node.rs) with a chosen network.
2. `create_genesis()` builds and mines a genesis block in-memory.
3. `ChainState::new` initializes the chain with genesis and a fresh `UtxoSet`.
4. `Miner::mine_and_attach` is invoked either directly (tests) or via `mineblocks` over RPC:
   - `next_block_timestamp` uses MedianTimePast and network-adjusted time to pick a valid timestamp.
   - `calculate_next_target` in [difficulty.rs](file:///c:/ferrous/src/consensus/difficulty.rs#L63-L145) computes the next difficulty target from the previous header and timestamp.
   - `Miner` iterates nonce values until `BlockHeader::check_proof_of_work` succeeds.
5. `ChainState::add_block` validates the new block and connects it:
   - Validates difficulty via `validate_difficulty`.
   - Validates structure and PoW via `validate_block`.
   - Updates `UtxoSet` if the chain is extended or reorganized.

### RPC Request Handling

1. Incoming HTTP requests are handled by `RpcServer::run`.
2. The body is parsed as JSON and forwarded to `handle_json_rpc`.
3. Depending on `method`, the server:
   - Reads from `ChainState` (e.g. `getblockchaininfo`, `getbestblockhash`, `getblock`).
   - Mutates `ChainState` via mining (`mineblocks`).
4. Responses are serialized using the structs in [methods.rs](file:///c:/ferrous/src/rpc/methods.rs) and returned as JSON.

## Thread Safety and Concurrency Model

The node uses a simple concurrency model:

- `ChainState` is shared between RPC handlers and the `Miner` via `Arc<Mutex<ChainState>>`.
- All block additions and reads that require consistency are done by locking the same `Mutex`.
- `Miner` itself is wrapped in an `Arc` and is internally immutable aside from its configuration.

Implications:

- There is no fine-grained concurrency; all state mutations are serialized through a single lock.
- This is acceptable for a single-node test/regtest environment and greatly simplifies correctness.
- Scaling to high throughput or multi-threaded mining would require a more sophisticated synchronization strategy.

## Dependencies and Rationale

Key external dependencies (see [Cargo.toml](file:///c:/ferrous/Cargo.toml#L7-L17)):

- `sha2`, `ripemd`: Implement `hash160` and `sha256d` without custom crypto.
- `secp256k1`: Provides ECDSA verification for transaction signatures.
- `num-bigint`: Used in difficulty and work calculations.
- `serde`, `serde_json`: JSON encoding/decoding for RPC.
- `tiny_http`: Minimal HTTP server for the RPC interface.
- `clap`: Command-line parsing for the example node binary.

Design choices:

- **No unsafe code**: Enforced at the crate level in [lib.rs](file:///c:/ferrous/src/lib.rs#L1-L3).
- **In-memory state**: `ChainState` and `UtxoSet` use `HashMap`s, prioritizing clarity over persistence. This is suitable for tests and experimentation.
- **Explicit serialization**: Custom `Encode`/`Decode` traits in [serialize.rs](file:///c:/ferrous/src/primitives/serialize.rs) avoid relying on `serde` for consensus-critical binary formats.

## Limitations and Future Work

- No persistent storage; all chain and UTXO state is lost on restart.
- No P2P networking; RPC is the only interface, so nodes cannot sync or gossip.
- Network parameters differ only in difficulty behavior and target block time; all networks currently share a single genesis block.
- Address formats are raw scriptPubKeys (no Base58 or Bech32 address layer).

These limitations are intentional to keep the implementation focused on consensus and validation while remaining approachable for readers and contributors.

