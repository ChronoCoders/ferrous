# Phase 5 — Ring CT Implementation Plan

Status: **Planning** (2026-06-01). Builds on `PRIVACY.md` (the design spec). This document
is the *sequenced implementation plan* and records the foundational decisions.

## Foundational decisions (2026-06-01)

| Decision | Choice | Rationale |
|---|---|---|
| EC group | **Ristretto255** via `curve25519-dalek` 4.x | Mature, widely used; prime-order group avoids Ed25519 cofactor pitfalls; the Bulletproofs ecosystem is built on it. |
| Range proofs | **Bulletproofs** (original, not BP+) | Use the reviewed `bulletproofs` crate (Ristretto). Amends the `PRIVACY.md` "Bulletproofs+" to plain BP — slightly larger proofs, far lower crypto risk. |
| Delivery | **Staged: 5a → 5b → 5c** | Each stage is small, testable, reviewable, and independently deployable. Avoids one giant unreviewable change. |
| Authorization | Unchanged: **Dilithium signs the tx body** | Hybrid model from `PRIVACY.md` — funds stay PQ-safe even if the EC privacy layer is broken. |

**Custom-crypto flag:** there is no turnkey audited Rust **CLSAG** crate. CLSAG (5b) must be
implemented on top of `curve25519-dalek` primitives. This is the one unavoidable hand-rolled
component and is the primary target for the **Phase 6 external audit**. Commitments (5a) and
range proofs (5a) use established crates; only the ring-signature glue is bespoke.

## Crate additions

- `curve25519-dalek` 4.x — Ristretto255 group/scalar ops, `RistrettoPoint::from_uniform_bytes` for hash-to-point.
- `bulletproofs` 4.x — aggregated range proofs over Ristretto.
- `merlin` — transcripts (required by `bulletproofs`).
- Cleanup: **remove the dangling `secp256k1`** dependency from `Cargo.toml` (no longer referenced in `src/`).

## Prerequisite (before 5a code): size & verification-cost model

A v2 tx (11-member ring + BP range proof + Dilithium sig) is ~**8–15 KB**. At
`MAX_BLOCK_WEIGHT = 40M` that is ~2,600–5,000 tx/block by size, but **verification cost**
(ring sig + range proof per tx) is the real throughput ceiling, not bytes. Deliverable:
a benchmark (`examples/`) measuring per-tx verify time, and a recalibrated block/mempool
policy. **Must complete before building 5a.** (CLAUDE.md flags this as a hard prerequisite.)

---

## Stage 5a — Confidential Amounts

Hide **amounts**; sender and recipient remain visible. Smallest self-contained privacy win.

- **EC keys enter the wallet/address.** To encrypt the amount + blinding factor to a recipient
  you need an ECDH shared secret, which requires the recipient to publish a Ristretto **view
  key**. So 5a front-loads part of the dual-key wallet work: the address format gains a
  Ristretto view key alongside the Dilithium spend authority. (Full one-time *stealth* output
  keys are deferred to 5c.)
- **Pedersen commitments** `C = xG + aH`. `H` derived nothing-up-my-sleeve from `G`
  (hash-to-point). Blinding factor `x` derived deterministically from the wallet seed.
- **Aggregated Bulletproofs** range proof over all outputs, proving each amount ∈ [0, 2⁶⁴).
- **Balance check** `Σ C_in − Σ C_out − fee·H = 0` (commitment-homomorphic; fee is public).
- **Tx output** carries `{commitment, encrypted_amount}`; **input** references a committed UTXO.
- UTXO set stores `commitment` (per `PRIVACY.md` `UtxoEntry`).
- Spend authorization still Dilithium; **no ring yet** — input still names its real predecessor.
- **Deliverable:** confidential-amount transactions verified end-to-end; reviewer + audit-of-glue.

## Stage 5b — Sender Privacy (CLSAG + key images)

- **CLSAG ring signatures**, ring size 11, decoys via gamma distribution over output age.
- **Key images** `I = x·H_p(P)`, stored in a new `CF_KEYIMAGE` column family.
- **Double-spend detection moves to the key-image set** for v2 inputs (replaces UTXO-spent
  semantics). Mempool + consensus reject duplicate key images.
- Input gains `{ring_members, pseudo_commitment}`; witness gains `ring_signature`.
- **This is the bespoke-crypto stage** — CLSAG implemented on dalek primitives; heaviest review.

## Stage 5c — Recipient Privacy (stealth addresses)

- **One-time output keys** `P = H(rA)G + B` (Monero-style stealth).
- **Wallet output scanning**: view key scans every output to detect received funds; spend key
  authorizes. Completes the dual-key model started in 5a.
- Recipient address no longer appears on-chain; outputs become unlinkable.

---

## Verification order (consensus + mempool, cheapest-first)

`Dilithium sig → key-image uniqueness → ring signature → range proof` (per `PRIVACY.md`).

## Testnet reset 7

Tx v2 is a flag-day wire/consensus change. A reset is acceptable on testnet (decided in
CLAUDE.md). Each stage can ride its own reset, or 5a/5b/5c can accumulate behind a v2 activation
height — to be decided when 5a stabilizes.

## Risks (carried from CLAUDE.md Ring CT risk summary)

1. **Tx size explosion** — modeled in the prerequisite step; block/mempool policy recalibrated.
2. **Verification cost / DoS surface** — cheapest-first verify order; per-tx verify benchmark gates policy.
3. **UTXO growth** — commitments + key images grow the set; pruning strategy needed before mainnet.
4. **Bespoke CLSAG** — external audit in Phase 6 is mandatory before mainnet.
5. **Wallet migration** — dual-key model and new address format; flag-day reset on testnet.

## Open items to decide as 5a stabilizes
- Whether stages share one v2 activation or each gets its own testnet reset.
- Encrypted-amount scheme details (ECDH KDF, domain separation) — fix during 5a.
- Pruning strategy for the key-image set and committed UTXOs (before Phase 6).
