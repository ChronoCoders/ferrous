# Ferrous Network Consensus Specification

This document specifies the Ferrous Network consensus rules: block and transaction formats, proof-of-work, difficulty adjustment, UTXO and script validation, chain selection, and network parameters.

## Block Structure

Blocks consist of:

- 88-byte block header.
- Compact-size VarInt for transaction count.
- Serialized transactions (with witnesses).

The block header is defined in [block.rs](file:///c:/ferrous/src/consensus/block.rs#L45-L54):

- `version: u32`
- `prev_block_hash: Hash256` (32 bytes)
- `merkle_root: Hash256` (32 bytes, over legacy txids)
- `timestamp: u64` (seconds since UNIX epoch)
- `n_bits: u32` (compact difficulty target)
- `nonce: u64`

The total encoded header size is fixed at 88 bytes (`HEADER_SIZE`).

The block hash is:

- `hash = sha256d(header_bytes)`

where `header_bytes` is the little-endian encoding of the fields above.

## Transaction Format

Transactions are defined in [transaction.rs](file:///c:/ferrous/src/consensus/transaction.rs#L40-L48):

- `version: u32`
- `inputs: Vec<TxInput>`
- `outputs: Vec<TxOutput>`
- `witnesses: Vec<Witness>`
- `locktime: u32`

### Inputs

`TxInput`:

- `prev_txid: Hash256` (32-byte little-endian transaction id)
- `prev_index: u32` (output index)
- `script_sig: Vec<u8>` (script for legacy spends / height commitment)
- `sequence: u32`

Coinbase inputs are recognized by:

- `prev_txid == [0u8; 32]`
- `prev_index == 0xFFFF_FFFF`

### Outputs

`TxOutput`:

- `value: u64` (satoshis)
- `script_pubkey: Vec<u8>` (locking script, e.g. P2PKH or P2WPKH)

The total supply is bounded by `MAX_MONEY`:

- `MAX_MONEY = 21_000_000 * 100_000_000`

### Witnesses

`Witness`:

- `stack_items: Vec<Vec<u8>>`

Witness serialization:

- VarInt count of stack items.
- For each item, a VarInt length plus raw bytes.

Witnesses are stored in `Transaction::witnesses` with the same length as `inputs`.

### Encodings and Identifiers

The transaction implements custom binary encoding via `Encode`/`Decode`.

- `encode_without_witness()` encodes version, inputs, outputs, and locktime.
- `encode_with_witness()` additionally appends the witness vector.

Identifiers:

- `txid = sha256d(encode_without_witness())`
- `wtxid = sha256d(encode_with_witness())`

## Proof of Work

### Target Representation

The compact target (`n_bits`) is decoded by `BlockHeader::target()`:

- `exponent = n_bits >> 24` (high byte)
- `mantissa = n_bits & 0x00FF_FFFF`
- `target = mantissa * 256^(exponent - 3)` (subject to bounds)

Constraints:

- `mantissa < 0x0080_0000` (non-negative encoding)
- `exponent <= 32` (fits into 256 bits)

The full 256-bit target is represented as `U256([u8; 32])` in little-endian order.

### Hash and Validation

The block hash is computed as:

- `hash = sha256d(header_bytes)`

Proof-of-work validation (`check_proof_of_work`):

- Convert `hash` to `U256` (little-endian).
- Compute `target` from `n_bits`.
- Accept if `hash_value <= target`.

## Difficulty Adjustment

Difficulty retargeting is implemented in [difficulty.rs](file:///c:/ferrous/src/consensus/difficulty.rs#L63-L145) and depends on `ChainParams`.

Inputs:

- `prev_header`: previous block header.
- `current_timestamp`: proposed timestamp for the new block.
- `params.target_block_time`: target spacing in seconds.
- `params.max_target`: maximum allowed difficulty target.
- `params.difficulty_adjustment`: enable/disable adjustment.

Steps:

1. Decode the previous target from `prev_header.n_bits`.
2. If `params.difficulty_adjustment` is false, reuse the previous target.
3. Compute `actual_timespan = current_timestamp - prev_header.timestamp` (saturating).
4. Clamp timespan:
   - `min_timespan = target_block_time / 4`
   - `max_timespan = target_block_time * 4`
   - `actual_timespan = clamp(actual_timespan, min_timespan, max_timespan)`
5. Scale target linearly:
   - `new_target = prev_target * actual_timespan / target_block_time`
6. Ensure bounds:
   - If `new_target` is zero, set to minimum (1).
   - If `new_target > max_target`, set to `max_target`.
7. Convert back to compact form (`u256_to_compact`) and then to `U256`.

Effectively, Ferrous uses a per-block proportional adjustment with a factor bounded to \[1/4, 4\] of the target spacing.

`validate_difficulty` recomputes the expected target for each block and rejects blocks whose decoded target does not match.

## UTXO and Monetary Rules

### Block Validation

`validate_block` in [validation.rs](file:///c:/ferrous/src/consensus/validation.rs#L32-L90) enforces:

1. At least one transaction.
2. First transaction must be coinbase; no other coinbase transactions.
3. Merkle root of `txid`s matches `header.merkle_root`.
4. Proof-of-work is valid.
5. Block weight does not exceed `MAX_BLOCK_WEIGHT = 4_000_000`.
6. Each transaction passes `check_structure`.
7. No duplicate `txid`s within the block.

### Coinbase Reward and Supply

The block subsidy is defined by `calculate_subsidy`:

- `INITIAL_SUBSIDY = 50 * 100_000_000`
- `HALVING_INTERVAL = 840_000` blocks
- `subsidy(height) = INITIAL_SUBSIDY >> (height / HALVING_INTERVAL)`

`validate_coinbase_reward`:

- Computes `max_reward = subsidy + block_fees`.
- Sums all coinbase outputs.
- Ensures the coinbase value is:
  - ≤ `MAX_MONEY`.
  - ≤ `max_reward`.

### Coinbase Maturity

`COINBASE_MATURITY = 100` confirms:

- Coinbase outputs must not be spent until at least 100 blocks after inclusion.
- Enforcement is implemented inside the UTXO and spending logic.

### Double-Spend Prevention

The `UtxoSet` tracks unspent outputs for the active chain and:

- Fails if a transaction attempts to spend a non-existent or already spent outpoint.
- Applies and rolls back UTXOs when reorganizations occur.

## Timestamp and MedianTimePast

`validate_timestamp` enforces:

- For non-genesis blocks, the block timestamp must be strictly greater than the MedianTimePast of the last up to 11 blocks.
- The timestamp must not be more than 2 hours (7200 seconds) into the future compared to the local system time.

MedianTimePast is computed both in `validate_timestamp` and in `ChainState::median_time_past` to drive mining.

## Script Validation

Scripts are executed by `execute_script` in [engine.rs](file:///c:/ferrous/src/script/engine.rs#L32-L216).

Supported features:

- Push operations: direct pushes (1–75 bytes) and `OP_PUSHDATA1/2/4`.
- Small integer constants: `OP_0`, `OP_1NEGATE`, `OP_1`–`OP_16`.
- Stack operations: `OP_DUP`, `OP_DROP`.
- Hashing: `OP_HASH160`.
- Comparisons: `OP_EQUAL`, `OP_EQUALVERIFY`.
- Verification: `OP_VERIFY`.
- Signature checks: `OP_CHECKSIG`, `OP_CHECKSIGVERIFY`.
- Termination: `OP_RETURN` (immediate failure).

Errors are captured as `ScriptError` variants (e.g. `StackUnderflow`, `InvalidOpcode`, `VerifyFailed`).

### P2PKH and P2WPKH

Patterns:

- **P2PKH scriptPubKey**:
  - `OP_DUP OP_HASH160 <pubkey_hash> OP_EQUALVERIFY OP_CHECKSIG`
- **P2WPKH scriptPubKey**:
  - `0 <20-byte pubkey_hash>` (witness program)

The engine uses `ScriptContext` to compute the correct `sighash` and verify signatures based on:

- The full transaction.
- The input index.
- The set of spent outputs.

Signatures are currently verified using `secp256k1` ECDSA as a placeholder, with a 64-byte signature format similar to Schnorr.

## Witness Commitments

`validate_witness_commitment` enforces a BIP141-style witness commitment when any transaction has witness data:

- The coinbase must contain an `OP_RETURN` output with:
  - `0x6a` (OP_RETURN)
  - `0x24` (36-byte push)
  - Magic bytes `0xaa 0x21 0xa9 0xed`
  - 32-byte witness commitment.
- The commitment is:
  - `sha256d(witness_merkle_root || reserved_value)`
  - `witness_merkle_root` is computed from `wtxid`s of all non-coinbase transactions.
  - `reserved_value` is taken from the first element of the coinbase witness stack if present, or zero.

If witness data exists but no valid commitment is found, the block is rejected.

## Chain Selection and Reorganization

`ChainState` maintains:

- A map of block hash → `BlockData` (header, transactions, height, cumulative work).
- The current tip hash.

Chain selection:

- Each block’s cumulative work is computed from the difficulty target.
- The best chain is the one with the greatest cumulative work.

Reorganization:

- When a competing chain with greater work is found, `ChainState`:
  - Identifies the fork point.
  - Rolls back UTXOs along the old branch.
  - Applies UTXOs along the new branch.

This ensures deterministic selection of the heaviest chain.

## Network Parameters

Network parameters are defined in [params.rs](file:///c:/ferrous/src/consensus/params.rs#L3-L24) via `ChainParams`:

- `target_block_time: u64`
- `max_target: U256`
- `difficulty_adjustment: bool`
- `allow_min_difficulty_blocks: bool` (reserved for future behavior)

Networks:

- **Mainnet**
  - `target_block_time = 150`
  - `max_target = MAINNET_MAX_TARGET`
  - `difficulty_adjustment = true`
  - `allow_min_difficulty_blocks = false`
- **Testnet**
  - `target_block_time = 150`
  - `max_target = TESTNET_MAX_TARGET`
  - `difficulty_adjustment = true`
  - `allow_min_difficulty_blocks = true` (not yet used in consensus rules)
- **Regtest**
  - `target_block_time = 1`
  - `max_target = REGTEST_MAX_TARGET`
  - `difficulty_adjustment = false` (fixed difficulty)
  - `allow_min_difficulty_blocks = true`

All networks currently share the same genesis block parameters; differences arise only from difficulty behavior and target spacing.

