# Ferrous Network Roadmap

This document outlines the phased development and feature roadmap for the Ferrous Network.

## Phase 1: Foundation (Current - Alpha)

- **Consensus**: SHA256d Proof-of-Work, 150s blocks, difficulty adjustment.
- **Networking**: P2P protocol, headers-first sync, block relay.
- **Storage**: RocksDB persistence for chain state and UTXOs.
- **Interface**: JSON-RPC and TUI Dashboard.

## Phase 2: Wallet & Transaction Management

- **Wallet Implementation**:
  - Key management (BIP32/BIP39 HD wallets).
  - Address generation (P2PKH, P2WPKH).
  - Transaction building and signing.
- **Transaction Features**:
  - Fee estimation.
  - Replace-by-Fee (RBF) logic.
  - Mempool prioritization.

## Phase 3: Privacy Layer (Beta)

- **New Transaction Version (v2)**: Introduce Ring Confidential Transactions.
- **Cryptography**:
  - Integrate `dalek-cryptography` for Bulletproofs+ and Ristretto.
  - Integrate `pqc_dilithium` for post-quantum signatures.
- **Consensus Rules**:
  - Add validation logic for range proofs and ring signatures.
  - Implement key image storage and double-spend checking.

## Phase 4: Mainnet Launch

- **Security Audits**: External review of consensus and privacy code.
- **Testnet Reset**: Final wipe of testnet chain.
- **Genesis Block**: Launch of the permanent mainnet.
- **Mining**: Release open-source miner software.

## Phase 5: Ecosystem Growth

- **Block Explorer**: Web interface to view blocks and transactions.
- **Light Client (SPV)**: Mobile-friendly wallet protocol.
- **Exchange Integration**: API support for exchanges.

## Long-Term Vision

- **Lattice Commitments**: Replace EC Pedersen commitments with fully PQ-safe alternatives.
- **Sidechains**: Support for smart contracts via Layer 2 solutions.
