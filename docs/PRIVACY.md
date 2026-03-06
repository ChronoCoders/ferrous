# Ferrous Privacy Specification

This document outlines the planned architecture for the privacy and post-quantum cryptographic upgrade of the Ferrous Network.

## Overview

Ferrous will implement a hybrid cryptographic model combining:
- **Ring Confidential Transactions (RingCT)** for privacy (hiding sender, amount, and recipient).
- **CRYSTALS-Dilithium** for post-quantum authorization (ownership proof).

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
- **Algorithm**: Pedersen Commitments.
  - `C = xG + aH`
- **Blinding**: Deterministic derivation from wallet seed.
- **Balance Check**: `Sum(In) - Sum(Out) - Fee = 0`.

### 3. Range Proofs (Hiding Amounts)
- **Algorithm**: Bulletproofs+ (aggregated).
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
- **pseudo_commitment**: Commitment to the input amount for the ring signature.

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

1.  **Phase 1**: Implement P2P & Consensus (Current).
2.  **Phase 2**: Wallet Integration & Transaction v2 Format.
3.  **Phase 3**: Privacy Layer (RingCT + Dilithium) Activation on Testnet.
4.  **Phase 4**: Mainnet Activation via Soft Fork (Block Height).
