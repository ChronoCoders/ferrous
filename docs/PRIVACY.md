# Ferrous Privacy Specification

This document outlines the planned architecture for the privacy and post-quantum cryptographic upgrade of the Ferrous Network.

**Note**: CRYSTALS-Dilithium is planned to be implemented and stabilized before RingCT. They are not introduced simultaneously.

## Overview

Ferrous will introduce post-quantum authorization first, then privacy features:

- **CRYSTALS-Dilithium** for post-quantum authorization (ownership proof).
- **Ring Confidential Transactions (RingCT)** for privacy (hiding sender, amount, and recipient).

This ensures that even if a quantum computer breaks the privacy layer (EC-based), it cannot steal funds protected by the lattice-based signature scheme.

## Core Components

### 1. Ring Signatures (Privacy)
- **Algorithm**: CLSAG (Compact Linkable Spontaneous Anonymous Group).
- **Ring Size**: Fixed at 11 members.
- **Selection**: Decoys selected via Gamma Distribution based on output age.
- **Key Images**: Monero-style linkable key images to prevent double-spending.
  - `I = x * H_p(P)`
  - Stored in a dedicated RocksDB column family.

### 2. Commitments (Hiding Amounts)
- **Algorithm**: Pedersen Commitments over Ristretto255 (`curve25519-dalek`).
  - `C = commit(v, x) = v·G + x·H` — value `v` on the Ristretto basepoint `G`, blinding `x` on the independent generator `H`.
  - `G = RISTRETTO_BASEPOINT_POINT`; `H` is a nothing-up-my-sleeve hash-to-point of the domain tag `"Ferrous/H"` (independent of `G`, not derived from it). Matches `src/crypto/commitments.rs` (`pedersen_gens`/`h_generator`/`commit`).
- **Blinding**: Deterministic derivation from wallet seed.
- **Balance Check**: `Sum(C_in) - Sum(C_out) - fee·G = 0` — the public fee rides on `G` with zero blinding, summed into the coinbase exactly as v1 (`verify_balance` in `commitments.rs`).

### 3. Range Proofs (Hiding Amounts)
- **Algorithm**: Bulletproofs (aggregated), Ristretto255. (Amended from Bulletproofs+ on 2026-06-01 — use the reviewed `bulletproofs` crate; see `PHASE5_PLAN.md`.)
- **Function**: Proves that committed values are positive [0, 2^64) without revealing them.

### 4. Post-Quantum Authorization (Security)
- **Algorithm**: CRYSTALS-Dilithium (NIST FIPS 204).
- **Scope**: Signs the transaction body (inputs, outputs, fee, key images).
- **Placement**: Segregated Witness data.

## Transaction Structure (v2)

Privacy transactions will be introduced via a new version (`version = 2`).

### Inputs
- **prev_out**: Reference to a standard or stealth UTXO.
- **ring_members**: List of 10 decoy references + 1 real input.
- **pseudo_commitment**: Per-input **pseudo-output** commitment `C'_i = v_i·G + x'_i·H` (same value as the real input, fresh spender-chosen blinding). Balance is enforced on the pseudo-outputs (`Σ C'_i = Σ C_out + fee·G`, fee public); CLSAG signs the commitment-to-zero `C_real − C'_i = (x_real − x'_i)·H` at the real ring index. Excess model decided 2026-07-03 (Monero RingCT / CLSAG, per-input pseudo-outputs) — see `PHASE5_PLAN.md` BLOCKING-2 resolution.

### Outputs
- **stealth_address**: One-time destination key `P = H(rA)G + B`.
- **commitment**: Encrypted amount `C`.
- **encrypted_amount**: ECDH-encrypted amount for receiver decoding.

### Witness
- **ring_signature**: CLSAG signature proving knowledge of one private key in the ring.
- **range_proof**: Aggregated Bulletproof for all outputs.
- **dilithium_signature**: Authorization signature from the real spender.

## UTXO Model

After the upgrade, the UTXO set will track commitments instead of plaintext values.

```rust
struct UtxoEntry {
    commitment: [u8; 32],
    stealth_pubkey: [u8; 32],
    is_encrypted: bool,
}
```

## Mempool Rules

1.  **Duplicate Key Image**: Reject immediately (Double Spend).
2.  **Ring Validity**: Check that ring members exist in the chain.
3.  **Signature Order**: Verify Dilithium (fastest) -> Key Image -> Ring Sig -> Range Proof (slowest).

## Roadmap

1.  **Phase 1**: Foundation (PoW, UTXO, P2P, headers-first IBD). ✓
2.  **Phase 2**: Parallel IBD completion + testnet reset. ✓
3.  **Phase 3**: Wallet integration (BIP39, Shamir SSS, encryption). ✓
4.  **Phase 4**: Post-Quantum Cryptography (CRYSTALS-Dilithium).
5.  **Phase 5**: Privacy Layer (RingCT + CLSAG).
6.  **Phase 6**: Security audit + Mainnet launch.
