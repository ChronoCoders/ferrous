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

### Results (DONE 2026-06-13, `examples/verify_bench.rs`, commit `53c39c5`)

Benchmark is dev-deps-only (`curve25519-dalek` 4, `curve25519-dalek-ng` 4 +
`bulletproofs` 4 + `merlin` 3) — it does **not** ship in the node binary. No consensus
code touched. Measured on seed4 (Vultr 1 vCPU, single-thread, RandomX miner paused so
timings aren't skewed by core contention). v1, Ristretto, and Bulletproofs figures are
real measurements; CLSAG is an estimate built from the measured size-2 multiexp primitive
(no CLSAG impl exists yet).

| Measurement | seed4 (1 vCPU) |
|---|---|
| v1 raw Dilithium verify | 0.338 ms |
| v1 tx verify (1 input) | 0.352 ms → 2 844 tx/s |
| v1 tx verify (10 input) | 3.61 ms → 277 tx/s |
| v1 tx weight (1in/2out) | 21 641 units → 1 848 tx/block, 0.65 s/block |
| Ristretto255 scalar mul (var-base) | 0.0525 ms |
| Ristretto multiexp (size 2) | 0.0626 ms |
| BP range proof verify (m=1) | 2.73 ms |
| BP range proof verify (m=2, aggregated) | 4.77 ms |
| CLSAG estimate (ring N=11) | 1.39 ms |
| **v2 tx verify (1in/2out, est.)** | **6.50 ms → 154 tx/s** |
| v2 tx verify (2in/2out, est.) | 7.89 ms → 127 tx/s |

Cost composition of the 6.50 ms v2 tx: **range proof 4.77 ms (73%)**, CLSAG 1.39 ms (21%),
Dilithium 0.34 ms (5%). v2 is **~18× slower to verify than v1** (154 vs 2 844 tx/s) — the
headline DoS/throughput shift.

### Policy decision: keep `MAX_BLOCK_WEIGHT = 40M` for the v2 launch

At 40M with v2 tx ≈ 8–15 KB (weight = 4×size), a block holds ~650–1 220 v2 tx and verifies
in **4–8 s** on the 1-vCPU reference node — ~3–5% of the 150 s interval, leaving ~19–35×
realtime headroom for IBD. **Size is the binding constraint, not verification.** The
verification ceiling at a 15 s/block budget (10% of interval) is ~2 308 tx ≈ **113M weight
units**, far above 40M — so no weight increase is warranted, and 40M must **not** exceed
~113M on this hardware without the optimizations below.

Three levers to fix before/with 5a:
1. **Witness discount (decision pending)** — if ring sigs + range proofs ride in the witness
   (weight 1× not 4×), the same 40M admits ~3 250 v2 tx/block but block-verify rises to ~21 s
   (still < interval, less IBD headroom). This discount choice directly sets the real cap and
   must be decided when the v2 wire format is fixed in 5a.
2. **Batch range-proof verification** — the range proof is 73% of per-tx cost; Bulletproofs
   verifies a whole block's proofs together far cheaper than per-tx. Highest-value validation
   optimization; implement for block validation.
3. **Parallel verify** — figures are single-thread/1-vCPU; rayon over inputs/txs + the planned
   4-vCPU mainnet nodes ≈ 4× the ceiling.

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

### 5a status (2026-06-14)

Types, commitment scheme, range proofs, `validate_transaction_v2`, wire format, and tests
landed in `7c3618a` (4 tests + 65 prior = 69 pass). v2 is **not wired** into block validation,
mempool, UTXO storage, or the network decode path — it is referenced only within its own module.
rust-reviewed on `7c3618a`: the crypto plumbing (generator threading, commit/prove/verify
consistency, range-proof binding, sighash field coverage, wire round-trip) is correct; the items
below were raised as **hard prerequisites before 5b wires any v2 path into consensus**. The v1
decode unbounded-allocation DoS (shared latent pattern) was fixed separately — `MAX_TX_INPUTS` /
`MAX_TX_OUTPUTS` / `MAX_WITNESS_ITEMS` caps (1000) now bound every attacker-controlled count in
`Transaction::decode`, `Witness::decode`, and `TransactionV2::decode`.

## 5b Prerequisites (must resolve before wiring v2 into consensus)

- **BLOCKING-1 — `fee_commitment` is unsound.** `validate_transaction_v2` checks
  `Σ in == Σ out + fee_commitment` where `fee_commitment` is an arbitrary attacker-supplied
  Ristretto point with no range proof and no proof its blinding (H) component is zero. With
  inputs committed at blinding 0, the equation is satisfiable for any output values — the fee's
  value-generator coefficient is whatever the attacker picks (including negative ⇒ inflation).
  Fix: use a **public `fee: u64` ⇒ `fee·G`** (zero blinding), summed into the coinbase exactly as
  v1. `verify_balance(.., fee: u64)` already implements this correctly; `verify_balance_committed`
  with an unconstrained `fee_commitment` must not be carried into consensus. A full CT design
  requires an explicit excess / commitment-to-zero kernel and a range-proofed fee.
- **BLOCKING-2 — input-blinding-zero excess undefined.** Because 5a spends public v1 UTXOs
  (blinding 0), the only place the output blinding sum can be absorbed is the fee term. 5b must
  define where the output blinding sum lives (a real CT excess/kernel, or a change-output
  construction) and **prove it commits to zero** in the value generator.
- **N1/N2 — batch range-proof verification + cached gens.** `validate_transaction_v2` verifies
  range proofs one-by-one and rebuilds `BulletproofGens::new(64,1)` on every call. Before consensus
  exposure: use Bulletproofs **batched/aggregated** verification (the range proof is ~73% of v2
  verify cost per the benchmark) and cache the generators in a `OnceLock` (as `h_generator` does).
  Also bound output count / total proof bytes per tx.
- **N7 — unambiguous v1/v2 discriminator.** A v1 `Transaction` with `version == 2` and a
  `TransactionV2` both lead with a `version = 2` `u32`. They are safe today only because they decode
  on separate Rust paths; before any shared decode path (mempool/network/block) introduce a
  distinct discriminator (distinct version number or explicit type tag).

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
