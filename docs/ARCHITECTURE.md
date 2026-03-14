# Ferrous Network Architecture

This document describes the internal architecture of the Ferrous Network node, including modules, data flow, key abstractions, and the concurrency model.

## Module Overview

The crate root is defined in [lib.rs](file:///c:/ferrous/src/lib.rs#L1-L12) and exposes these top-level modules:

- [consensus](file:///c:/ferrous/src/consensus/mod.rs)
- [dashboard](file:///c:/ferrous/src/dashboard/mod.rs)
- [mining](file:///c:/ferrous/src/mining/mod.rs)
- [network](file:///c:/ferrous/src/network/mod.rs)
- [primitives](file:///c:/ferrous/src/primitives/mod.rs)
- [rpc](file:///c:/ferrous/src/rpc/mod.rs)
- [script](file:///c:/ferrous/src/script/mod.rs)
- [storage](file:///c:/ferrous/src/storage/mod.rs)
- [wallet](file:///c:/ferrous/src/wallet/mod.rs)

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

### network

P2P Networking stack:

- [manager.rs](file:///c:/ferrous/src/network/manager.rs): `PeerManager`, the central hub for P2P connections and message dispatch.
- [peer.rs](file:///c:/ferrous/src/network/peer.rs): `Peer` state machine, connection handling, and message queues.
- [protocol.rs](file:///c:/ferrous/src/network/protocol.rs): Wire protocol definitions, message types (`Version`, `Inv`, `Block`, etc.), and serialization.
- [sync.rs](file:///c:/ferrous/src/network/sync.rs): `SyncManager`, handling headers-first synchronization and block downloads.
- [relay.rs](file:///c:/ferrous/src/network/relay.rs): `BlockRelay`, managing inventory announcements and transaction propagation.
- [discovery.rs](file:///c:/ferrous/src/network/discovery.rs): Peer discovery, address exchange (`GetAddr`/`Addr`), and bootstrapping.
- [keepalive.rs](file:///c:/ferrous/src/network/keepalive.rs): Connection health monitoring (Ping/Pong) and dead peer detection.
- [addrman.rs](file:///c:/ferrous/src/network/addrman.rs): `AddressManager` for storing and selecting peer addresses.

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

### storage

Persistent storage and indexes:

- [db.rs](file:///c:/ferrous/src/storage/db.rs): RocksDB wrapper and column family wiring.
- [blocks.rs](file:///c:/ferrous/src/storage/blocks.rs): Block/header storage and indexes.
- [chain_state.rs](file:///c:/ferrous/src/storage/chain_state.rs): Persistent chain tip and best-header tracking.
- [utxo.rs](file:///c:/ferrous/src/storage/utxo.rs): UTXO storage.

### wallet

Wallet management and transaction building:

- [manager.rs](file:///c:/ferrous/src/wallet/manager.rs): Wallet state and address management.
- [builder.rs](file:///c:/ferrous/src/wallet/builder.rs): Transaction construction.

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

### Network Recovery and Diagnostics

To ensure robustness, the node includes a dedicated recovery system:

- **RecoveryManager**: Monitors network health and detects partitions.
  - Tracks active peer count and last block reception time.
  - Automatically attempts reconnection if isolated (0 peers for >5m or stale tip >30m).
  - Strategies: Query known good peers (AddressManager), fallback to seed nodes, or force full reconnect.
- **NetworkStats**: Aggregates metrics (bytes sent/recv, message counts, ban scores) for diagnostics.
- **Diagnostics**: Provides detailed peer health reports via RPC.

### RpcServer

Defined in [server.rs](file:///c:/ferrous/src/rpc/server.rs#L8-L12), `RpcServer` owns:

- `Arc<Mutex<ChainState>>` for synchronized access to the chain.
- `Arc<Miner>` for mining operations.
- `Arc<PeerManager>` for P2P network control.
- `Arc<RecoveryManager>` for network health monitoring.
- `Arc<NetworkStats>` for metrics.
- A `tiny_http::Server` instance bound to the configured address.

Responsibilities:

- `run()` accepts HTTP requests and dispatches JSON-RPC calls.
- Implements control, wallet, mining, and network methods including `getblockchaininfo`, `getblockhash`, `getblock`, `getbestblockhash`, `getmininginfo`, `getnetworkinfo`, `getpeerinfo`, `getconnectioncount`, `getnewaddress`, `getbalance`, `listunspent`, `listaddresses`, `sendtoaddress`, `generatetoaddress`, `resetnetwork`, and `stop`.

## Data Flow

### P2P Networking

1. `PeerManager` is initialized and starts listening on the configured port.
2. `NetworkListener` accepts incoming TCP connections and spawns a thread for each.
3. `PeerDiscovery` runs in the background, finding new peers via `getaddr` and `addr` messages.
4. `KeepaliveManager` sends periodic `ping` messages to ensure connection health.
5. Incoming messages are dispatched by `PeerManager` to specialized handlers:
   - `SyncManager`: Handles `headers` and `block` downloads during Initial Block Download (IBD).
   - `BlockRelay`: Propagates new blocks and transactions via `inv`/`getdata`.
   - `PeerDiscovery`: Updates the address book with new peer information.

#### SyncManager (Headers-First IBD)

`SyncManager` tracks progress using a `SyncState` enum:

- `Idle`
- `DownloadingHeaders`
- `DownloadingBlocks`
- `Synced`

### Mining and Block Application

1. The node is started from [examples/node.rs](file:///c:/ferrous/examples/node.rs) with a chosen network.
2. `create_genesis()` builds and mines a genesis block in-memory.
3. `ChainState::new` initializes the chain with genesis and a fresh `UtxoSet`.
4. `Miner::mine_and_attach` is invoked either via CLI mining (`--mine`) or via RPC (`mineblocks` / `generatetoaddress`):
   - `next_block_timestamp` uses MedianTimePast and network-adjusted time to pick a valid timestamp.
   - `calculate_next_target` in [difficulty.rs](file:///c:/ferrous/src/consensus/difficulty.rs#L63-L145) computes the next difficulty target from the previous header and timestamp.
   - `Miner` iterates nonce values until `BlockHeader::check_proof_of_work` succeeds.
5. `ChainState::add_block` validates the new block and connects it:
   - Validates difficulty via `validate_difficulty`.
   - Validates structure and PoW via `validate_block`.
   - Updates `UtxoSet` if the chain is extended or reorganized.
6. If valid, the new block is relayed to peers via `BlockRelay::broadcast_block`.

### RPC Request Handling

1. Incoming HTTP requests are handled by `RpcServer::run`.
2. The body is parsed as JSON and forwarded to `handle_json_rpc`.
3. Depending on `method`, the server:
   - Reads from `ChainState` (e.g. `getblockchaininfo`, `getbestblockhash`, `getblock`).
   - Mutates `ChainState` via mining (`mineblocks`).
   - Queries `PeerManager` (e.g. `getpeerinfo`, `addnode`).
4. Responses are serialized using the structs in [methods.rs](file:///c:/ferrous/src/rpc/methods.rs) and returned as JSON.

## Thread Safety and Concurrency Model

The node uses a simplified concurrency model relying on `std::sync`:

- `ChainState` is protected by a global `Mutex`, shared between RPC, Mining, and Networking threads.
- `PeerManager` uses internal `Mutex` locks for its peer map and component references (`Relay`, `Sync`, `Discovery`).
- Background threads:
  - **RPC Server**: One thread per request (via `tiny_http`).
  - **P2P Listener**: One thread accepting connections.
  - **Peer Threads**: One thread per connected peer for handshake and message reading.
  - **Maintenance Threads**: Separate threads for `PeerDiscovery` (30s loop) and `KeepaliveManager` (60s loop).
- `Miner` is stateless/immutable configuration wrapped in `Arc`.

Implications:

- **Global Lock Contention**: `ChainState` lock is the primary bottleneck. Long validation or reorganization holds the lock, blocking RPC and P2P processing.
- **Deadlock Risk**: Care is taken to avoid holding the `PeerManager` lock while calling into `ChainState`, or vice-versa.
- **Scaling**: Planned improvements include `RwLock` for `ChainState` (parallel reads), a separate RPC thread pool to isolate long-running requests, and further work on parallel IBD (multi-peer download + ordered apply).

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
- **In-memory hot path**: `ChainState` maintains an in-memory view of the active chain for validation and mining, while persistence is provided via RocksDB.
- **Explicit serialization**: Custom `Encode`/`Decode` traits in [serialize.rs](file:///c:/ferrous/src/primitives/serialize.rs) avoid relying on `serde` for consensus-critical binary formats.

## Limitations and Future Work

- `ChainState` is guarded by a global `Mutex` and is a known bottleneck under load (RPC timeouts can occur during heavy sync/mining).
- Parallel IBD is in progress: Phase 1 headers-first state machine is deployed; multi-peer download + ordered apply are planned next.
- Address format and wallet/RPC ergonomics are evolving and will change as Dilithium and later privacy features are integrated.
