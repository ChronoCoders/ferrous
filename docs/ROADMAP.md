# Ferrous Network Roadmap

## Phase 1: Foundation (Current)
- SHA256d PoW, 150s block time, per-block difficulty adjustment
- Full P2P stack: handshake, headers-first IBD, block relay, peer discovery
- RocksDB persistent storage
- JSON-RPC API (20+ methods) and TUI Dashboard + Monitor
- Parallel IBD: headers-first state machine deployed 

## Phase 2: Parallel IBD Completion + Testnet Reset
- BlockDownloadQueue with work-stealing multi-peer download
- BlockApplyBuffer with sequential ordered validation
- Testnet reset with 4 vCPU servers, 5+ nodes all mining
- RPC threading fix: RwLock + separate thread pool

## Phase 3: Wallet Integration
- BIP39 seed phrase (12-24 words)
- Shamir's Secret Sharing recovery (M-of-N)
- Wallet encryption (KDF + AEAD, replacing XOR obfuscation)
- RPC authentication

## Phase 4: Post-Quantum Cryptography
- CRYSTALS-Dilithium (NIST FIPS 204) — replaces ECDSA
- New address format for Dilithium public keys
- Block/mempool size policy redesign (Dilithium sigs 2.4-4.6KB)
- Hard fork coordination

## Phase 5: Privacy Features
- Ring Confidential Transactions (RingCT) + CLSAG
- Bulletproofs+ range proofs
- Pedersen commitments
- Key image storage and double-spend prevention
- Only after Dilithium is stable — never simultaneously

## Phase 6: Mainnet Launch
- Independent security audit
- Final testnet reset
- Genesis block creation
- Open-source miner release

## Long-Term
- Block explorer (vanilla JS, no framework)
- Public faucet
- Exchange backend + market maker bot
- Light client (SPV)
