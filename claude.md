# Ferrous Network — Claude Briefing Document
Last updated: 2026-03-19

## Previously Blocking Bugs — NOW FIXED

**Bug 1 — Write-lock starvation (FIXED, commit `15e61d0`):** `blocks: HashMap` in `chain.rs` replaced with `LruCache<Hash256, BlockData>` capped at 2048 entries. Blocks not in cache are served from RocksDB. Write-lock hold time per `add_block` is now O(1). All per-block `println!` calls across `relay.rs`, `manager.rs`, and `sync.rs` also removed (commits `48c6688`, `2c000b5`).

**Bug 2 — Difficulty runaway (FIXED, commit `b3ecab9`):** Distinct `max_target` values set per network (mainnet: 32 leading zero bits; testnet: 16 leading zero bits; regtest: trivial). `allow_min_difficulty_blocks` disabled for testnet — this caused the difficulty floor to snap to near-zero during solo-mining windows.

**Note on equal-height convergence fix (`0bb3890`):** The `should_download` logic was confirmed correct. The convergence was blocked by Bug 1 (getdata starvation), not by the convergence logic itself.

---

## Known Truths (Hard Invariants — Always Valid)

1. Deploy order: seed4 first → wait for peer count > 0 → seed1. Never simultaneously.
2. Build on servers: `cargo build --release --example node` on server after git checkout. No fmt/clippy/test on servers.
3. fmt/clippy on developer machine: must pass before any deploy.
4. Dual mining can now be re-enabled: LRU cache fix deployed (`15e61d0`), difficulty floor fixed (`b3ecab9`), all per-block println! removed. The 3-cycle convergence verification test is the next required step before Phase 2. seed4 `--mine` should be confirmed re-enabled after convergence test passes.
5. WAL sync:false: crash-safe but not fsync-safe. Hard power loss can lose recent writes.
6. RPC is loopback only: `127.0.0.1:8332`. No external access without SSH tunnel.
7. Both nodes are symmetric: full node + seed + miner. No role separation.

---

## Claude's Role

Claude is the review and approval gate between Chronocoder and the implementation agents.

- Chronocoder does not proceed without sign-off from the full team.
- Claude writes prompts, reviews implementations, and maintains documentation.
- Communication language: English 
- No technical debt — nothing is deferred without explicit risk flagging.
- Nothing moves until: Claude Code proposes → Claude reviews → Chronocoder approves → TRAE verifies → real test passes.

---

## Project Overview

Ferrous Network is a Bitcoin-derived layer 1 blockchain written entirely in Rust.

- Ticker: FRR | Tagline: "Bitcoin, re-forged"
- ~13,750 lines of code across 63 files
- GitHub: ChronoCoders/ferrous (public)
- Solo developer: Chronocoder
- Current status: Testnet live, 2 nodes running; seed1 mining, seed4 mining disabled pending blocks HashMap LRU fix

### Consensus Parameters

- PoW algorithm: SHA256d (current) → RandomX (planned, before mainnet)
- Block time: 150 seconds
- Difficulty adjustment: ±1% per block
- Block reward: 50 FRR
- Halving interval: every 840,000 blocks
- Maximum supply: 21,000,000 FRR
- Smallest unit: frsats
- Coinbase maturity: 100 blocks
- Ledger model: UTXO
- Magic bytes: MAINNET 0xD9B4BEF9 | TESTNET 0x0B110907 | REGTEST 0xFABFB5DA

### PoW Algorithm Decision (2026-03-15)

Decision: migrate from SHA256d to RandomX before mainnet.

Rationale:
- SHA256d allows Bitcoin ASICs to mine Ferrous, which conflicts with the decentralization mission.
- RandomX is CPU-friendly, memory-hard, and has been ASIC-resistant since 2019 (Monero).
- A custom PoW algorithm was explicitly rejected — unaudited custom cryptography is dangerous.
- Implementation: `randomx-rs` crate (Rust wrapper around the reference C++ implementation).
- Timing: applied during the testnet reset, after Phase 2 and Phase 3 are stable.
- Impact: the hash function call in miner.rs is replaced, difficulty parameters recalibrated.
- Long-term: RandomX parameters will be periodically adjusted to maintain ASIC resistance, following Monero's practice.

### Cryptography (Current State)

- Signatures: ECDSA (DER encoding)
- Addresses: Base58Check
- Privacy: transparent UTXO — no Ring CT yet

### Cryptography Roadmap (Not Yet Implemented)

- CRYSTALS-Dilithium (NIST FIPS 204) — replaces ECDSA
- Ring CT + CLSAG — confidential transactions

---

## Infrastructure

### Live Nodes

| Node                   | IP              | Provider         | Role         |
|------------------------|-----------------|------------------|--------------|
| seed1.ferrous.network  | 45.77.153.141   | Vultr, New York  | Mining node  |
| seed4.ferrous.network  | 45.77.64.221    | Vultr, Frankfurt | Mining node  |

### Not Yet Deployed

- seed2.ferrous.network — Akamai, Atlanta
- seed3.ferrous.network — DigitalOcean, San Francisco
- seed5.ferrous.network — Akamai, Singapore
- seed6.ferrous.network — Radore, Istanbul (planned; good geographic position between Frankfurt and Singapore)

### Network Topology (Finalized)

All nodes are identical in role: full node + seed + miner. No role separation. Symmetric architecture with no single point of failure. Roles may diverge organically as the network grows post-mainnet.

### Service Configuration

- Both nodes run under systemd as `ferrous.service` with `Restart=on-failure`.
- Binary path: `/root/ferrous/target/release/examples/node`
- Source on servers: `~/ferrous/` — git repository. Git fetch + checkout is the canonical deploy method. scp is reserved for emergency single-file fixes only.

Canonical deploy command:
```bash
ssh root@IP "cd ~/ferrous && git fetch origin main && git checkout -f origin/main && ~/.cargo/bin/cargo build --release --example node 2>&1 && systemctl restart ferrous"
```

seed1 ExecStart:
```
/root/ferrous/target/release/examples/node --network testnet --datadir /root/ferrous/data --wallet /root/ferrous/wallet.dat --rpc-addr 127.0.0.1:8332 --p2p-addr 0.0.0.0:8333 --seed-nodes 45.77.64.221:8333 --mine
```

seed4 ExecStart:
```
/root/ferrous/target/release/examples/node --network testnet --datadir /root/ferrous/data --wallet /root/ferrous/wallet.dat --rpc-addr 127.0.0.1:8332 --p2p-addr 0.0.0.0:8333 --seed-nodes 45.77.153.141:8333 --mine
```

---

## Deploy Workflow

Emergency scp-based deploy (single file):
```bash
scp src/network/foo.rs root@45.77.153.141:/root/ferrous/src/network/foo.rs
scp src/network/foo.rs root@45.77.64.221:/root/ferrous/src/network/foo.rs
ssh root@IP "cd ~/ferrous && ~/.cargo/bin/cargo build --release --example node 2>&1"
ssh root@IP "systemctl restart ferrous"
ssh root@IP "journalctl -u ferrous -n 20 --no-pager"
```

### CRITICAL: Deploy Order

Never restart both nodes simultaneously. Doing so causes peer disconnection, independent mining, and divergent chains.

Correct procedure:
1. Deploy and restart seed4 (45.77.64.221) first.
2. Wait for seed4 to reconnect to seed1 and confirm peer count > 0.
3. Deploy and restart seed1 (45.77.153.141).
4. Verify both nodes are on the same chain by comparing `getblockhash` at a shared height.

---

## RPC

RPC call pattern:
```bash
ssh root@IP 'printf "{\"jsonrpc\":\"2.0\",\"method\":\"METHOD\",\"params\":[PARAMS],\"id\":1}" | curl -s -X POST http://127.0.0.1:8332 -H "Content-Type: application/json" -d @-'
```

Available RPC methods:
```
getblockchaininfo, getblockhash, getblock, getbestblockhash, getblockcount,
getmininginfo, mineblocks, generatetoaddress, getnewaddress, getbalance, listunspent,
listaddresses, sendtoaddress, sendrawtransaction, getnetworkinfo, getpeerinfo,
getconnectioncount, getnetworkhealth, getrecoverystatus, forcereconnect, resetnetwork, stop
```

Chain verification commands (run to confirm current state before trusting stored values):
```bash
ssh root@45.77.153.141 "curl -s -X POST http://127.0.0.1:8332 -H 'Content-Type: application/json' -d '{\"jsonrpc\":\"2.0\",\"method\":\"getblockchaininfo\",\"params\":[],\"id\":1}'"
ssh root@45.77.64.221 "curl -s -X POST http://127.0.0.1:8332 -H 'Content-Type: application/json' -d '{\"jsonrpc\":\"2.0\",\"method\":\"getblockchaininfo\",\"params\":[],\"id\":1}'"

# Same-chain check — hashes must match at the same height
ssh root@45.77.153.141 "curl -s -X POST http://127.0.0.1:8332 -H 'Content-Type: application/json' -d '{\"jsonrpc\":\"2.0\",\"method\":\"getblockhash\",\"params\":[160000],\"id\":1}'"
ssh root@45.77.64.221 "curl -s -X POST http://127.0.0.1:8332 -H 'Content-Type: application/json' -d '{\"jsonrpc\":\"2.0\",\"method\":\"getblockhash\",\"params\":[160000],\"id\":1}'"
```

---

## Architecture

```
src/
├── consensus/     block.rs, chain.rs, difficulty.rs, merkle.rs, params.rs,
│                  transaction.rs, utxo.rs, validation.rs
├── dashboard/     stats.rs, ui.rs (ratatui TUI)
├── mining/        miner.rs (rayon parallel PoW)
├── network/       addrman, batch, connection, diagnostics, discovery, dos,
│                  handshake, keepalive, listener, manager, mempool, message,
│                  peer, protocol, ratelimit, recovery, relay, security,
│                  stats, sync, validation
├── primitives/    hash.rs, serialize.rs, varint.rs
├── rpc/           methods.rs, server.rs (tiny_http, 1MB body limit)
├── script/        engine.rs, opcodes.rs, sighash.rs
├── storage/       blockchain_db.rs, blocks.rs, chain_state.rs, db.rs, utxo.rs
└── wallet/        address.rs, builder.rs, keys.rs, manager.rs
examples/node.rs   — main node binary
```

---

## Complete Fix History

### Audit Fixes (Before First Deployment)

1. `BlockHeader::work()` — correct `floor(2^256/(target+1))` calculation
2. Difficulty adjustment — removed broken `.min(1)` cap, added ±1% bounds
3. Block validation wiring — timestamp, coinbase height (skip genesis), coinbase reward
4. UTXO rollback in reorg — CF_UNDO column family
5. ECDSA signature DER encoding standardization
6. `validate_p2pkh` stack check + mainnet address prefix
7. Network 32MB payload cap + RPC 1MB body limit
8. `.unwrap()` removal in DB iterator
9. `Transaction::encoded_size()`, dead code cleanup, recovery network param
10. Wallet key XOR obfuscation
11. Reorg double-apply (critical) — `did_reorg` flag
12. Reorg coinbase inflation (critical) — `validate_coinbase_reward` enforced during reorg reconnect

### Deployment and Network Fixes

13. Zombie/duplicate inbound connections — skip duplicate check for trusted IPs
14. `store_header_only` implemented — saves to CF_HEADERS in RocksDB
15. `add_block` DB fallback for parent — falls back to `block_store.get_block()`
16. `handle_headers` — transitions to `DownloadingBlocks` state when >= 2000 headers received
17. `block_received` moved after `add_block` in relay.rs; chain lock dropped before calling to prevent deadlock
18. Removed `request_headers_force` from orphan handler
19. `handle_headers` — removed duplicate `start_sync` call from the >= 2000 path
20. Deployed missing storage files — CF_UNDO column family and undo data methods
21. seed4 data wipe #1 (diverged chain)
22. INV rate limiter bypass for trusted peers in manager.rs
23. seed4 data wipe #2 (diverged at block 21888)
24. INV flood fix — `mineblocks` RPC now announces only the last block
25. IBD batch continuation — `DownloadingBlocks` retains `peer_id` and continues on empty pending queue
26. `handle_inv` checks `is_syncing()` before calling `request_headers_force`
27. Diversity check bypass for trusted IPs
28. Inbound handshake fix — receive VERSION → send VERSION → send VERACK → wait VERACK
29. Handshake timeout extended from 10s to 30s
30. Duplicate connection prevention in `connect_to_peer`
31. Block serving from RocksDB fallback in `handle_getdata_inner`
32. `handle_headers` with >= 2000 headers also calls `request_blocks_from_headers`
33. `--mine` flag added to node binary
34. `Network` enum `#[derive(Clone)]`
35. `Transaction::decode` — witness field made optional, fixes UnexpectedEof on blocks with no witness data
36. `SyncManager.block_received` — resumes sync instead of going Idle when local height < peer height
37. Orphan handler triggers `start_sync` when SyncManager is Idle
38. `handle_getheaders` — falls back to height 1 when no locator hash matches

### Security and Consensus Fixes (Claude Code Audit, 2026-03-14/15)

39. Double-spend within single transaction — `HashSet<OutPoint>` per-tx guard in `apply_block_to_utxo`
40. Reorg common ancestor detection — hard error instead of silent break at genesis sentinel
41. Unbounded deserializer allocation — `MAX_VEC_DECODE_BYTES = 32MB` cap in serialize.rs
42. Script engine panic on empty stack — returns `Ok(false)` instead of unwrapping
43. P2WPKH signature validation — full ECDSA verify, hash160 check, sighash computation
44. Magic bytes validation — `verify_magic()` called on every incoming message before dispatch
45. Timestamp validation in headers-first sync — `validate_timestamp()` called in `handle_headers` loop
46. Block height index overwritten during reorg — `store_block_no_index()` used for side chains
47. Fee underflow — `checked_sub()` replaces unchecked subtraction
48. Transaction version validation — version 0 and version > 2 are rejected
49. Difficulty adjustment timestamp ordering — explicit check before `saturating_sub`
50. Mempool bounds — `MEMPOOL_MAX_ENTRIES = 5000` rejection guard
51. Mempool TOCTOU race — atomic check-then-insert under single lock
52. Connection history TTL — 60-second time-based pruning in security.rs
53. Recovery chain linearity — `prev_block_hash` linkage validated in `recover_from_storage`
54. `MAX_FUTURE_BLOCK_TIME` — promoted to `pub const` at module level
55. Transaction version 0 / > 2 — `InvalidVersion` error in `check_structure`
56. Coinbase-only block witness merkle — documented behavior
57. Difficulty timestamp ordering — explicit guard before subtraction
58. `start_sync` height guard — stale VERSION height no longer aborts sync
59. `handle_headers` locator walk — direct DB lookup replaces broken in-memory walk
60. `prev_hash` height resolution — three-level fallback: locator → block_meta
61. Canonical chain check removed — `add_block` handles cumulative work comparison
62. `ts_window` VecDeque — reduced from ~22,000 DB reads per batch to ~10
63. `cached_prev` — 0 DB reads on sequential header processing path
64. `store_headers_batch` — 2000 individual writes collapsed into 1 commit
65. `sendrawtransaction` RPC — hex decode, deserialize, mempool submit, txid return
66. RocksDB WAL — `sync: false` applied globally (WAL enabled, fsync disabled)
67. RPC threading fix — mining loop now uses read lock for PoW, write lock only for `add_block`. Lock held for ~ms instead of ~150s. Both nodes remain ONLINE consistently in monitor.

### Session 2026-03-18 to 2026-03-19 (Claude Code)

78. Per-block println! removal across `relay.rs`, `manager.rs` (dispatch + block-worker), `sync.rs` — 19+ calls converted to `log::debug!/warn!/info!`. Eliminates journald buffer saturation while holding chain write lock. Commits `48c6688`, `2c000b5`.
79. LRU cache replacing full-chain HashMap (`15e61d0`) — `blocks: HashMap<Hash256, BlockData>` replaced with `LruCache<Hash256, BlockData>` capped at 2048. Recovery walks last 2048 blocks reading `BlockMeta` from RocksDB (O(min(chain_height, 2048)) startup). Reorg chain-walking and UTXO apply/revert fall back to `block_store` on cache miss. `peek()` used throughout to avoid `&mut` borrow conflicts.
80. Distinct `max_target` per network + disable testnet `allow_min_difficulty_blocks` (`b3ecab9`) — mainnet: 32 leading zero bits (Bitcoin difficulty-1), testnet: 16 leading zero bits (65536× easier), regtest: unchanged trivial. `allow_min_difficulty_blocks: false` for testnet — this flag is designed for epoch-based retargeting and causes difficulty runaway in per-block adjustment systems.
81. Measured hash rate in mining dashboard — `MiningEvent` gains `hashes_tried: u64` + `elapsed_secs: f64`. Miner tracks total hashes via shared `AtomicU64` across all rayon workers. `BlockInfo` computes `hash_rate = hashes_tried / elapsed_secs`. `MiningStats.hash_rate` field is now live (was dead code, always 0.0). Dashboard `render_mining_stats` displays auto-scaled H/s / kH/s / MH/s.
82. Orphan handler calls `request_headers_force` (`b3c44c6`) — `relay.rs` `Err(ChainError::OrphanBlock)` arm was silent. Added sync trigger for fork resolution.
83. env_logger initialized (`214c17e`) — `env_logger::Builder::from_env(...).init()` added to `examples/node.rs` main(). All `log::*` calls were silently dropped before this fix.
84. handle_getheaders locator DB fallback (`68b6f43`) — locator hash matching in `handle_getheaders` now falls back to `block_store.get_block_meta()` for blocks outside the 2048-entry LRU window.
85. difficulty: timestamp < prev no longer hard error (`52bf6f2`) — `calculate_next_target` replaced `current_timestamp < prev.timestamp → Err` with `saturating_sub`. Bitcoin consensus only requires timestamp > MTP, not > prev.
86. Testnet reset 2026-03-19 — chains had blocks with `n_bits=0x207fffff` (genesis difficulty) in the middle of the chain, mined by old code without `validate_difficulty` in `add_block`. Both nodes wiped and restarted fresh. Nodes now converging correctly via reorgs with no difficulty errors.

### Session 2026-03-16 to 2026-03-18 (Claude Code)

68. Block dispatch worker (`f063b90`) — block messages enqueued into `mpsc::sync_channel(1024)`; dedicated `block-dispatch-worker` thread processes them. Poll loop never blocks on `add_block`. Root fix for 1-block-per-30s TCP backpressure issue.
69. RPC non-blocking chain lock (`b76c88c`) — `try_read` with timeout/fallback in `server.rs`; RPC no longer hangs during heavy sync or mining.
70. MTP validation DB fallback (`46ad292`) — `chain.rs` now loads prev headers from DB for side-chain blocks. Eliminates `TimestampTooOld` errors on fork sync.
71. Equal-height fork convergence (`0bb3890`) — `should_download` updated to `pw > local_work || (pw == local_work && fork_start_height.is_some())` in both the non-empty and empty headers paths in `sync.rs`. Fixes the simultaneous-restart divergence scenario where both nodes mined one solo block and never converged.
72. sccache on servers + monitor network summary (`d109ad4`) — `.cargo/config.toml` sets `rustc-wrapper = "sccache"` on both nodes; incremental recompiles now skip RocksDB rebuild. Monitor footer shows network send/recv rates.
73. Monitor FORK indicator fix (`6c757da`) — FORK only shown when `seed1.blocks == seed4.blocks && seed1.best_hash != seed4.best_hash`. Height difference alone (propagation lag) no longer triggers FORK.
74. Monitor `conn=0` fix (`29ce0a4`) — `getnetworkinfo` in `rpc/server.rs` now reads `self.peer_manager.get_peer_count()` instead of the never-populated `stats.current_connections`.
75. Monitor recent blocks 5 → 10 (`9c8517b`) — `fetch_recent_blocks` count raised from 5 to 10.
76. Monitor local time (`ef9ea0f`) — footer timestamp switched from UTC `SystemTime` to `chrono::Local::now()` for the developer's local timezone.
77. Monitor last block age consistency (`7f9689a`) — `block_time: u64` (unix timestamp) added to `RecentBlock`; both nodes' ages computed from a single `now()` at render time, eliminating cosmetic divergence caused by poll-time skew.

---

## Current Status (2026-03-19)

Always run the chain verification commands above to confirm state before trusting the values below.

Last known state (may be stale):
- seed1: ~400+ blocks (fresh testnet, reset 2026-03-19), mining, active
- seed4: ~400+ blocks (fresh testnet, reset 2026-03-19), mining, active
- Both nodes converging on same chain via reorgs — no difficulty errors
- Git HEAD: `d9bb960` (debug: log difficulty mismatch — remove before Phase 2)
  - Actually cleaned up: reverted debug log, pending commit
- All 6 convergence fixes deployed and working:
  1. `b3c44c6` relay.rs orphan handler calls `request_headers_force`
  2. `214c17e` env_logger initialized in node.rs main()
  3. `68b6f43` handle_getheaders locator falls back to block_meta DB
  4. `52bf6f2` difficulty: saturating_sub replaces hard error for timestamp < prev
  5. `0bb3890` should_download: equal-work fork triggers download
  6. Testnet reset 2026-03-19: wiped corrupt chain data (blocks had invalid n_bits from old code without difficulty validation)
- sccache enabled on both nodes
- Both nodes mining (--mine on both)
- Reorgs happening correctly (1-12 block reorgs resolved automatically)

NEXT: 3-cycle convergence verification test, then Phase 2 (WAL perf, RandomX).

---

## Monitoring

Both servers run `/root/ferrous/monitor.sh` via nohup, every 5 minutes.

```bash
ssh root@45.77.153.141 "tail /root/ferrous/sync_log.txt"
ssh root@45.77.64.221 "tail /root/ferrous/sync_log.txt"
```

A standalone Ratatui TUI monitor is available at `examples/monitor.rs`. It uses SSH tunnels on ports 18331 and 18332 and displays seed1 and seed4 side by side with real-time blocks, difficulty, hashrate, peers, and recent block data.

---

## Website

`ferrous-network.html` — single file, dark terminal aesthetic, no framework.

Corrections applied:
- Dilithium and Ring CT marked as roadmap items, not current features
- seed2, seed3, seed5 marked as PLANNED
- Node count shown as 2
- ECDSA shown as the current signature scheme
- Max supply and halving interval added
- RPC method count shown as 16 (now 21 with recent additions — update pending)

---

## Session Summary (2026-03-19) — Convergence Test

### What Was Tested

Re-enabled `--mine` on seed4 and observed behavior when seed4 reconnected to seed1. A real fork occurred: seed4 restarted, its handshake to seed1 timed out (seed1 was mining and accepting no new connections fast enough), and both nodes mined independently for ~6 minutes before seed1 was manually restarted to force reconnection.

### Equal-Height Fix Confirmed Correct

seed1's SyncManager correctly detected the fork (`fork_start=Some(...)`) and set `download=true`. The fix to `should_download` is logically sound. The convergence was blocked not by the logic but by getdata starvation — seed1 could not serve blocks to seed4 quickly because the chain write lock was held too often by the mining thread.

### Write-Lock Starvation — New Critical Bug Confirmed

During the ~6 minute disconnection, seed1's rayon mining threads called `add_block` many times per second (difficulty near-zero). All external threads — RPC, logging, getdata — were starved. seed1 mined 1357 blocks while appearing completely frozen to the outside. seed4 could not download seed1's longer chain because seed1 could not serve getdata responses fast enough to keep up with its own growth rate.

### Difficulty Runaway — Confirmed

After solo mining at near-zero difficulty, seed1 reached height ~254,323 from ~252,908 in ~8 minutes. The ±1% per-block adjustment cannot recover from this. When nodes reconnect, the longer chain (with more cumulative work but lower difficulty) wins, resetting difficulty to near-zero on both nodes.

### Recovery

Snapshot sync performed. Both nodes restored to seed1's chain at height ~254,334. RocksDB LOG.old files cleaned (289 files removed from both nodes). Both nodes restarted, connected, and confirmed on the same chain (`getblockhash` at shared height returns identical hash).

### Conclusion

The blocks HashMap LRU fix must precede any further dual-mining or convergence testing. Running both miners with the current architecture is not safe — it reliably produces write-lock starvation and runaway difficulty spirals under any disconnection scenario.

---

## Session Summary (2026-03-16 to 2026-03-18)

### TCP Backpressure Root Fix

The 1-block-per-30s download rate was caused by the peer poll loop blocking on `add_block` while the chain write lock was held. Chronocoder implemented a dedicated `block-dispatch-worker` thread with an `mpsc::sync_channel(1024)` queue. The poll loop enqueues and returns immediately; `add_block` is called only by the worker thread. WRITE_TIMEOUT also raised from 30s to 120s as a secondary band-aid. Both nodes now sync blocks at full network speed.

### RPC Lock Contention Fix

`src/rpc/server.rs` replaced blocking `read()` chain lock acquisition with `try_read()` + timeout fallback. RPC calls no longer hang when the chain write lock is held by the miner or sync thread.

### MTP Validation DB Fallback

`chain.rs` MTP window builder now falls back to `block_store.get_header()` for blocks in the DB but not in memory (side-chain blocks stored via `store_block_no_index`). Eliminated `TimestampTooOld` errors during fork sync.

### Equal-Height Fork Convergence Fix

Root cause of the simultaneous-restart divergence scenario: `should_download` in `sync.rs` evaluated `pw > local_work` — which is false when both nodes mined exactly one solo block. The fork_start_height fallback was unreachable because `peer_work` was `Some`. Fix applied to both the non-empty and empty headers paths: `pw > local_work || (pw == local_work && fork_start_height.is_some())`. Committed `0bb3890`. Deployed to both nodes. Verification (3 live cycles with both mining) is pending.

### sccache on Servers

`.cargo/config.toml` sets `rustc-wrapper = "sccache"`. Installed via `cargo install sccache` on seed1 and prebuilt binary on seed4 (seed4 OOM-killed during compile). Incremental recompiles on both nodes now skip the RocksDB rebuild (~10 min → ~30s for small changes).

### Monitor TUI Fixes (5 issues resolved)

- **FORK flickering** — only triggers when heights equal and hashes differ; height gap alone (propagation lag) was a false positive.
- **conn=0** — `getnetworkinfo` RPC was reading a never-populated `NetworkStats` field; fixed to use `peer_manager.get_peer_count()`.
- **Local time** — footer switched from UTC `SystemTime` to `chrono::Local::now()`.
- **Recent blocks** — count raised from 5 to 10.
- **Age consistency** — `block_time: u64` added to `RecentBlock`; both nodes' ages computed from the same `now()` at render time, eliminating cosmetic poll-time skew in the Recovery section.

---

## Session Summary (2026-03-11 to 2026-03-15)

### IBD Fix (3 parts)

- Locator match extended to DB lookup
- Parent DB fallback now loads real height and cumulative_work
- Minimal orphan pool added (HashMap, max 1000 entries)
- Result: seed4 began syncing from block 28

### Batch IBD

- SyncManager sliding window — 64-block batch getdata requests
- Throughput: 17 seconds/block → 24 blocks/20 seconds (24x improvement)

### RPC Fix

- POST body parse error corrected, GET fallback removed
- `getmininginfo` RPC method added

### Monitor TUI

- `examples/monitor.rs` — standalone Ratatui TUI
- SSH tunnel based (ports 18331/18332), seed1 and seed4 displayed side by side
- Real-time: block count, difficulty, hashrate, peer count, recent blocks

### Snapshot Sync (First Instance)

- seed4 could not close the IBD gap — serial IBD is a structural bottleneck
- seed1 data directory copied to seed4 (~40MB, ~6 minutes)
- `--mine` flag added to seed4

### Parallel IBD Phase 1 (Headers-First)

Explicit `SyncState` enum introduced: Idle, DownloadingHeaders, DownloadingBlocks, Synced.

- DB-aware locator using CF_HEADERS height index (hh: key scheme)
- Header validation: prev_hash linkage + PoW + difficulty via `calculate_next_target`
- Best header persisted via ChainStateStore using existing keys
- `header_height_map: HashMap<[u8;32], u32>` prepared for Phase 2

Architect review found 4 blocking issues, all fixed:
1. `Header[0]` prev_hash was picking the first DB match, not the highest-height match
2. DB-aware locator was using the block index instead of the header index
3. `add_block` DB-parent fallback was using placeholder height and work values
4. Block locator was not using the header height index

Deployed by Chronocoder (not TRAE).

### Security and Consensus Fixes (Claude Code Audit)

Claude Code identified and implemented fixes for 17 bugs. Critical items:
- Double-spend within a single transaction (HashSet guard)
- Reorg common ancestor detection (partial reorg fix)
- Unbounded deserializer allocation (32MB cap)
- Script engine panic on empty stack (Ok(false) instead of unwrap)
- P2WPKH signature validation (full ECDSA verify)
- Magic bytes validation enforced on every incoming message
- Timestamp validation in headers-first sync
- Block height index overwritten during reorg
- Fee underflow via checked_sub
- Transaction version validation
- Mempool bounds (5000 entries) + TOCTOU race fix
- Connection history TTL (60 seconds)
- Recovery chain linearity
- MAX_FUTURE_BLOCK_TIME constant
- RocksDB sync: false
- sendrawtransaction RPC

### Sync Performance Fixes (Claude Code)

During fork resolution, Claude Code identified 4 additional sync bugs:
1. `start_sync` height guard — stale VERSION height was aborting sync
2. `handle_headers` locator walk — broken in-memory walk replaced with direct DB lookup
3. 8-minute-per-batch bottleneck — per-write WAL fsync replaced with `store_headers_batch` (single commit)
4. `prev_hash` height resolution — headers-only DB entries were stalling; locator fallback added

### Fork Resolution

- Both nodes were restarted simultaneously during a deploy, causing divergent chains
- Claude Code sync fixes deployed
- Fork resolved via snapshot sync (second instance)
- Result: both nodes on the same chain, connected, both mining

### Key Lessons Learned (This Session)

- Serial IBD is a structural bottleneck — without parallel IBD, a new node can never catch up
- Deploy order is critical — seed4 first, seed1 second, never simultaneously
- `disable_wal` is not an acceptable shortcut — `sync: false` is the correct approach
- Claude Code is more capable than TRAE for complex multi-file fixes; use it accordingly
- TRAE is well suited for server operations (scp, systemctl, curl)
- The orphan pool is foundational — it should have been in place from day one; roughly one week was lost
- The GitHub repository is public — someone who found the IP addresses in the README attempted to connect (rejected by the diversity check)

### Fix Test Results

12 PASS, 1 UNTESTABLE at the time (double-spend — sendrawtransaction was not yet available; now added and testable)

- T1–T7: Sync fixes — PASS (verified on live nodes)
- T8: Double-spend — PASS (code verified); now directly testable via sendrawtransaction
- T9: MAX_VEC_DECODE_BYTES — PASS (code review)
- T10: Mempool bounds — PASS (code + unit test)
- T11: Connection history TTL — PASS (code review)
- T12: Recovery chain linearity — PASS (clean restart)
- T13: Magic bytes — PASS (unit test)

### Architect Phase 1 Review

- Verdict: Request changes (4 blocking issues identified)
- All blocking issues fixed and deployed
- Phase 2 sign-off: architect returns 2026-03-17

### Wallet Recovery Decision

- BIP39 seed phrase first (not yet in Ferrous — urgent)
- Shamir's Secret Sharing second (M-of-N split of the seed phrase)
- This combination does not exist in Bitcoin or Monero
- Implemented entirely in the wallet layer — no consensus changes required
- Implementation order: BIP39 → Shamir

### ECDSA and Quantum Resistance

- ECDSA is theoretically breakable by Shor's algorithm on a sufficiently capable quantum computer
- Practical timeline: 10–20 years; currently safe
- Ferrous proactively addresses this by planning CRYSTALS-Dilithium migration — this is a meaningful differentiator

### Other Decisions Made

- seed6 → Radore, Istanbul (good geographic position between Frankfurt and Singapore)
- RPC authentication is not urgent — loopback bind at 127.0.0.1, no external exposure currently
- Whitepaper: write only after all features are complete (after Dilithium and Ring CT)
- sccache installed locally; Windows Defender exclusion added
- Co-Authored-By (Claude) lines and the .claude directory removed from git history (rewrite performed)
- Server cleanup completed: tar.gz archives, backup directories, and TRAE.md removed from both servers

### Parallel IBD Architect Spec (Approved)

Phase 2 — BlockDownloadQueue:
- Work-stealing queue (not a fixed block range)
- Per-peer window: 64 blocks (fixed for now, adaptive later)
- Global in-flight cap: 512 blocks
- Peer drop: in-flight hashes returned to queue
- Timeout: re-request after 30 seconds

Phase 3 — BlockApplyBuffer:
- `buffer: HashMap<u32, Block>` (height → block)
- `next_expected` pointer
- Buffer cap: 1024 blocks (2x the global in-flight cap)
- Download pauses when buffer is full
- Parallel download, sequential apply

Orphan pool: resize to 2048 (4x the global in-flight cap of 512).
Testnet reset: not before Phase 2 and Phase 3 are stable.
Node count: remain at 2 seed nodes until Phase 2 and Phase 3 are stable.

---

## Pending Work

### Priority 1 — Blocking Phase 2

**[DONE] blocks HashMap LRU fix** — deployed `15e61d0`.
**[DONE] Difficulty runaway** — fixed `b3ecab9`.
**[DONE] Debug println! cleanup** — done `48c6688`, `2c000b5`.

**[PENDING] Deploy hash_rate dashboard fix** — `src/mining/miner.rs`, `src/dashboard/stats.rs`, `src/dashboard/ui.rs`. Commit and deploy to both nodes.

**[PENDING] Convergence verification (3-cycle test).** Re-enable `--mine` on seed4 if not already. Then:
1. Force a fork: stop seed4 for one block, reconnect.
2. Confirm both nodes converge to the same tip without manual intervention.
3. Repeat three times. All three must pass before Phase 2 starts.

### Priority 2 — Fix Before Testnet Reset (Not Blocking Phase 2)

- Wallet encryption — current XOR obfuscation in wallet/keys.rs is reversible; represents real security risk
- Mainnet address prefix — 0x6f (testnet) is hardcoded in wallet/address.rs
- RocksDB LOG.old cleanup — set `keep_log_file_num = 3` (289 files accumulated; manually cleaned 2026-03-19, will recur)
- Block hash caching — recomputed on every access, unnecessary work
- src/main.rs stub — 3 lines of "Hello world", confusing for anyone reading the codebase

### Priority 3 — Fix Before Mainnet (Not Before Phase 2 or Reset)

- RPC authentication — loopback-only binding is safe now, but authentication must be in place before any 0.0.0.0 exposure
- 189 mutex `.lock().unwrap()` calls — panics on poisoned mutex
- RocksDB tuning — bloom filters, compression, block cache
- Async I/O — current model is one thread per peer; will not scale beyond ~100 peers
- PeerManager late-init `Arc<Mutex<Option<T>>>` — circular dependency resolved via setters; should be redesigned
- Dual mempool sync — NetworkMempool and ChainState UTXO ledger are not synchronized

### Phase 2 (After Reorg Fix Verified)

- BlockDownloadQueue with work-stealing multi-peer download (architect spec approved above)
- BlockApplyBuffer with sequential ordered apply
- Orphan pool resize to 2048

### Phase 3 — Wallet Refactor

- BIP39 seed phrase (12–24 words)
- Shamir's Secret Sharing (M-of-N) on top of BIP39
- Wallet encryption (KDF + AEAD) — implemented together with BIP39, not separately
- Change address — dedicated change derivation path, not first-address reuse
- Coin selection — sort UTXOs descending by value

### Testnet Reset (After Phase 2 and Phase 3 Stable)

- RandomX PoW migration — replace SHA256d; use `randomx-rs` crate
- Upgrade servers to 4 vCPU
- Deploy all 5 nodes, all mining simultaneously
- Apply RocksDB LOG.old cleanup
- Recalibrate difficulty parameters for RandomX

### Long Term

- CRYSTALS-Dilithium (NIST FIPS 204) — before Ring CT, never simultaneously
- Ring CT + CLSAG — Bulletproofs mandatory
- Block and mempool size policy — must be designed before Dilithium (transaction size explosion)
- Block explorer (vanilla JS, no React)
- Public faucet
- Exchange backend + market maker bot

### Mainnet Launch Sequence (Finalized)

Testnet stable → reorg verified → Phase 2 + Phase 3 → testnet reset with RandomX → wallet refactor → Dilithium → Ring CT → independent audit → mainnet.

No mainnet before Dilithium and Ring CT. Hard fork post-mainnet is not acceptable. Single clean launch with all features ready.

---

## Architect Session Notes (2026-03-10)

### Team

- Chronocoder — architect and developer (design + implementation)
- Claude — review gate, prompt design, analysis, documentation
- TRAE — deployment, server operations, post-deploy verification
- Claude Code — deep implementation, complex multi-file fixes, codebase audits
- Auditor — security audit, independent verification
- Architect — architecture review (Dilithium and Ring CT phase)

### All Hands on Deck Protocol (Established 2026-03-15)

Nothing moves forward until all parties agree and the real test passes.

1. Claude Code proposes a fix.
2. Claude reviews carefully — no rubber stamping.
3. Chronocoder approves.
4. Deploy: seed4 first, confirm peer connection, then seed1.
5. Real test passes — not just a clean build.

No exceptions. No technical debt without an explicit risk flag.

### TRAE Rules

- Complex tasks: one fix per prompt, fully verified before the next prompt
- `cargo fmt -- --check` and `cargo clippy --all-features --all-targets -- -D warnings` must pass locally before any scp or deploy
- Server deploy sequence: scp → `cargo build --release --example node` → `systemctl restart ferrous`
- Do not run `cargo clippy` on servers — librocksdb compilation is too slow on a 1 vCPU instance
- TRAE.md is maintained locally only — delete it from servers if found
- Never add Co-Authored-By lines to commits
- Never run `git push`, `git commit`, or any git write operation without explicit instruction
- Report exact lines changed, build output, restart status, and log results for every fix
- Analysis-only prompts (read files, report findings, no changes) do not require deploy steps — state "analysis only, do not modify" explicitly in the prompt

### Dilithium and Ring CT Risk Summary (Architect Input)

1. Transaction size explosion — Dilithium signatures are 2.4–4.6KB; Ring CT adds multiple KB per transaction. Block and mempool policy must be redesigned before implementation begins.
2. Hard fork required — any node that is not upgraded will reject new blocks. Coordinated network-wide upgrade is mandatory.
3. Verification cost — Dilithium and Ring CT validation is significantly slower than ECDSA. This affects mining performance and increases the DoS attack surface.
4. Address format change — Base58Check is sized for 20-byte hashes; Dilithium public keys are much larger. A new address format is required.
5. UTXO growth — Ring CT commitments and range proofs are added per output. A pruning strategy is needed before deployment.
6. Migration — existing ECDSA addresses require a transition plan. Decision: flag-day reset on testnet is acceptable; mainnet approach is TBD.
7. Safe implementation sequence: fix consensus + IBD → block/mempool policy → Dilithium alone → Ring CT → audit → mainnet

### Wallet Recovery Strategy (Finalized)

Ferrous aims to solve the lost private key problem that Bitcoin does not address.

Decision: BIP39 seed phrase first, then Shamir's Secret Sharing on top.

BIP39 — 12–24 word mnemonic, user-friendly backup, industry standard. Not yet in Ferrous; this is an urgent gap.

Shamir's Secret Sharing — splits the seed phrase into M-of-N shares. Example: 3-of-5 produces 5 shares; any 3 recover the key. Mathematical guarantee: fewer than M shares reveal zero information.

This combination does not exist in Bitcoin or Monero. When implemented alongside Dilithium, Ferrous would offer: quantum-resistant keys + BIP39 mnemonic backup + Shamir recovery — a combination not available in any production blockchain today.

Implementation is entirely in the wallet layer. No consensus changes required. Order: BIP39 first, Shamir second.

### Testnet Stability Targets (Architect Recommendation)

At 150-second block time: 576 blocks per day.

Phase 1 — IBD stability (2–3 days, ~1,000–1,700 blocks): seed4 syncs from genesis without stalls or orphan storms.

Phase 2 — Soak test (2 weeks, ~8,000 blocks): sustained mining and P2P stability, no regression after node restarts.

Phase 3 — Confidence test (4–6 weeks, ~16,000–24,000 blocks): real-world uptime, consistent IBD after full data wipes, stable mempool and RPC behavior.

Minimum pass criteria:
- A fresh node syncs from genesis end-to-end three consecutive times
- No persistent orphan growth or recurring "No eligible next block" log entries
- After restarting both nodes, sync recovers automatically without manual intervention
- Block headers and block counts increase monotonically

Fastest reasonable proof: ~10,000 blocks over ~2 weeks with at least one successful full IBD wipe during that period.

### Key Architecture Decisions

- All 5 nodes: full node + seed + miner — symmetric, no role separation
- Orphan pool: required; minimal HashMap implementation is sufficient for now
- Mainnet: only after Dilithium + Ring CT + independent audit — no exceptions
- Dilithium first, Ring CT second — never implemented simultaneously
- TRAE complex task rule: one fix per prompt, verified before the next

---

## Key Lessons Learned

- Local unit tests are useless for P2P sync code — real TCP between real servers is required; regtest and local same-process testing hides sync bugs
- TRAE edits sometimes revert previously fixed bugs — always check handshake.rs after any relay or sync edits
- Every data wipe is a clean slate — do not hesitate when the chain diverges
- Short, scoped TRAE prompts work best
- Build time exceeding 2 minutes indicates a large change, not a build error
- Never restart both nodes simultaneously — deploy seed4 first, confirm peer connection, then seed1
- No shortcuts — never propose `disable_wal`, skip validation, or any safety trade-off without explicitly flagging it as a temporary hack with a concrete remediation plan; testnet status is not an excuse for unsafe code
- The reorg bug was missed in the Phase 1 review — SyncManager is downloader-only and never announces its own chain; fork → reconnect → no convergence; reorg must be verified in production before any new feature work begins
- All hands on deck as of 2026-03-15 — nothing moves until all parties agree and a real test passes; no rubber stamping
- The equal-height fork bug was caused by a single boolean operator: `pw > local_work` should have been `pw >= local_work when a fork is detected`; always trace the exact condition that prevents convergence, not just the symptom
- Mutual decision-making: when Claude offers help or a recommendation, Chronocoder's input must be taken before proceeding — no unilateral moves
- A node can appear frozen (no RPC, no logs) while still mining internally — write-lock starvation silences all reader threads but does not stop the rayon PoW threads; never assume a silent node is idle
- Never run dual mining without the blocks HashMap LRU fix in place — low difficulty + two miners + a single write lock is a guaranteed starvation scenario; the fix must come first, then the convergence test
