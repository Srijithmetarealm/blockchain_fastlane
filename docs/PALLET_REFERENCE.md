# FastLane Pallet — Reference

`pallet-fastlane` — a **Pre-Consensus Attestation** pallet. It adds a fast
threshold-signature agreement layer *before* block inclusion. A payload becomes
`PreConsensed` the instant an authorised committee reaches threshold, giving
instant UX confirmation, and is later made canonical by `Finalised`. It does
**not** replace block consensus or finality — it sits in front of it.

- Crate: [`pallets/fastlane`](../pallets/fastlane) · Source: [`src/lib.rs`](../pallets/fastlane/src/lib.rs)
- Tests: 46 (`cargo test -p pallet-fastlane`) — 43 pallet + 3 BLS proofs
- Live: deployed at `spec_version 105` on the projectfreedom.io testnet.

### Integration & Phase-2 status

- **§11 gateway (live).** FastLane exposes `EnsurePreConsensed`; `pallet-template`
  consumes it via `type PreConsensusGate = Fastlane` and its `do_something_gated`
  extrinsic refuses to act unless the referenced payload is PreConsensed. Enforced
  on-chain as of `spec_version 104`.
- **§14 real BLS aggregation (live).** BLS12-381 (TinyBLS381) aggregation runs
  on-chain as of `spec_version 105`: `register_bls_key` records an authority's
  BLS public key; `attest_bls` verifies each authority's BLS signature in-runtime
  (no host function) and, at threshold, sums them into a single 48-byte
  `BlsAggregate` — verified live (4 signatures → one 48-byte aggregate). The
  in-runtime verifier `verify_bls_aggregate` compiles into the WASM, so this
  deployed forklessly. The legacy bitmap `AggregatedAttestation` remains for the
  sr25519 path. Remaining: wiring each authority's **offchain worker** to submit
  `attest_bls` automatically (the crypto — `bls_sign_with_seed` — is done; the
  demo drives it via `scripts/bls-live-demo.js` + the `bls_gen` example).

---

## 1. Payload lifecycle

```
        submit()                 threshold reached            finalize() / auto
Submitted ───────────▶ Submitted ─────────────────▶ PreConsensed ──────────────▶ Finalised
                          │
                          │ expiry block passes, still Submitted
                          ▼
                       Expired
```

| Status | Meaning |
|--------|---------|
| `Submitted` | Registered, awaiting attestations. |
| `PreConsensed` | Threshold signatures collected — safe for a UI to treat as approved. |
| `Finalised` | State transition applied on-chain; canonical. |
| `Expired` | Expiry block passed before threshold. Terminal. |

`PreConsensed` payloads **cannot** expire.

---

## 2. Payload identity

```
payload_id = blake2_256(creator ++ nonce ++ domain ++ payload_hash)
```

The pallet stores only `payload_hash` (the `data_hash`), never the raw data.
Downstream pallets process the actual data and gate on the payload's status.

`Payload<T>` = `{ creator, created_at, expiry, domain: u32, payload_hash: [u8;32], nonce: u64 }`.

---

## 3. Storage

| Item | Type | Purpose |
|------|------|---------|
| `Authorities` | `BoundedVec<AccountId, MaxAuthorities>` | The committee. |
| `Threshold` | `u32` | Signatures required for pre-consensus (must be > 50%). |
| `AuthorityBonds` | `map AccountId => Balance` | Reserved slashable bonds. |
| `Payloads` | `map PayloadId => Payload<T>` | Registered payloads. |
| `Attestations` | `map PayloadId => BoundedVec<SignatureRecord>` | Per-authority signatures (pruned on finalise). |
| `Statuses` | `map PayloadId => Status` | Lifecycle state. |
| `Nonces` | `map AccountId => u64` | Replay protection for `submit`. |
| `PendingCount` | `u32` | Count of `Submitted` payloads (bounds pending load). |
| `ExpiryAt` | `map BlockNumber => BoundedVec<PayloadId>` | Per-block expiry index (O(1) expiry, no full scan). |
| `PreConsensedQueue` | `BoundedVec<PayloadId>` | Legacy FIFO of payloads awaiting auto-finalisation (superseded by `FinalizeAt`; still drained). |
| `FinalizeAt` | `map BlockNumber => BoundedVec<PayloadId>` | **§14** Per-block auto-finalisation schedule (`PreConsensed at b → due at b + FinalizeDelay`). |
| `AggregatedAttestations` | `map PayloadId => AggregatedAttestation` | **§14** compact aggregate; survives finalise. |
| `InclusionCommitments` | `map PayloadId => BoundedVec<InclusionCommitment>` | **§14** authorities' finalise-by-deadline promises. |

---

## 4. Extrinsics

| # | Call | Origin | Summary |
|---|------|--------|---------|
| 0 | `submit(nonce, expiry, domain, payload_hash)` | signed | Register a payload. Checks: `expiry > now`, `nonce == Nonces[who]`, domain allowed, pending limit. Emits `Submitted`. |
| 1 | `attest(payload_id, authority, signature, checks_hash)` | signed | Manual attestation (signer must equal `authority`). |
| 2 | `finalize(payload_id)` | signed (anyone) | Finalise a `PreConsensed` payload; runs `OnFinalised` hook, prunes signatures. Emits `Finalised`. |
| 3 | `set_authorities(new_set)` | root | Replace the committee (each must hold `MinAuthorityBond`). |
| 4 | `set_threshold(value)` | root | Set threshold; rejects `≤ 50%` (`ThresholdTooLow`) and `> authority_count` (`ThresholdTooHigh`). |
| 5 | `attest_unsigned(payload_id, authority_encoded, signature, checks_hash)` | none (OCW) | Unsigned attestation submitted by the offchain worker; validated by `ValidateUnsigned`. |
| 6 | `post_bond(amount)` | signed | Reserve funds as an authority bond. |
| 7 | `slash(authority, amount, proof)` | root | Slash a bond against verified proof; auto-evicts if bond drops below minimum. |
| 8 | `commit_inclusion(payload_id, deadline, signature)` | signed (authority) | **§14** Promise to finalise a `PreConsensed` payload by `deadline`. |

### Attestation message (signed by authorities)
```
msg_hash = blake2_256(payload_id ++ expiry.encode() ++ checks_hash)
```
Verified on-chain via `sr25519_verify`. `checks_hash` must equal
`OffchainChecks::compute_checks_hash(payload)`, so a signature only counts if the
authority ran the required validation.

Reaching `Threshold` signatures flips status to `PreConsensed`, builds the
aggregate, pushes onto `PreConsensedQueue`, and emits `PreConsensed`.

---

## 5. Events & errors

**Events:** `Submitted`, `PreConsensed`, `Finalised`, `Expired`,
`AuthoritiesUpdated`, `ThresholdUpdated`, `AuthoritySlashed`, `BondPosted`,
`AuthorityEvicted`, `AttestationsAggregated` (§14), `InclusionCommitted` (§14).

**Errors (selection):** `PayloadNotFound`, `NotAuthority`, `AlreadyAttested`,
`AlreadyPreConsensed`, `AlreadyFinalised`, `PayloadExpired`, `ThresholdNotMet`,
`InvalidSignature`, `ThresholdTooLow`/`ThresholdTooHigh`, `DuplicatePayload`,
`InvalidNonce`, `DomainNotAllowed`, `TooManyPendingPayloads`, `BondTooLow`,
`InsufficientBalance`, `SlashExceedsBond`, `MissingSlashProof`,
`NotPreConsensed` (§14), `AlreadyCommitted` (§14), `InvalidDeadline` (§14).

---

## 6. Offchain worker (§7)

Each authority node runs an OCW that, for every `Submitted` payload:
1. loads the payload and computes `checks_hash` via `OffchainChecks`
   (domain / nonce / balance checks live; KYC & rate-limit are extensible stubs);
2. signs `msg_hash` with its local `KEY_TYPE = "fast"` sr25519 key;
3. submits `attest_unsigned`, which `ValidateUnsigned` accepts only if the
   authority is registered, the payload is still `Submitted` and unexpired, the
   `checks_hash` matches, and the authority hasn't already attested.

An authority signs **only** if validation fully passes.

---

## 7. Integration gateway (§11)

Downstream pallets must not act on raw user calls — they gate on pre-consensus:

```rust
// trait implemented by Pallet<T>
FastLane::ensure_preconsensed(payload_id)?; // Ok iff PreConsensed or Finalised
```

This makes FastLane a mandatory approval gate for actionable state transitions.

---

## 8. Slashing (§10)

`slash(authority, amount, proof)` (root) burns from the reserved bond after
`validate_slash_proof`. Proof variants:

- `InvalidSignature(payload_id, sig)` — a recorded signature that fails verification.
- `DoubleSigning(id_a, sig_a, id_b, sig_b)` — the authority signed two different payloads.
- `BrokenInclusion(payload_id)` — **§14** the authority committed to include the
  payload, the deadline passed, and it is still not `Finalised`.

If the remaining bond falls below `MinAuthorityBond` the authority is
auto-evicted (`AuthorityEvicted`).

---

## 9. Optional Advanced Mode (§14 / Phase 2)

All three items are implemented and shipped in `spec_version 101`.

### 9.1 Auto-finalize batch — ✅ live
When a payload reaches threshold at block `b` it is scheduled in
`FinalizeAt[b + FinalizeDelay]`. Each block, `on_initialize` drains up to
`MaxAutoFinalizePerBlock` payloads due at `now` and finalises them (running
`OnFinalised` + pruning), rescheduling any overflow to the next block. Bounded so
a flood cannot stall the chain, and status-guarded so a manually-finalised
payload is never finalised twice. (The legacy `PreConsensedQueue` is still
drained for entries predating the schedule.)

### 9.2 Signature aggregation (storage reduction) — ✅ live
When threshold is reached, the individual signatures are collapsed into:
```rust
AggregatedAttestation { signer_bitmap: u64, digest: [u8;32], count: u32 }
```
- `signer_bitmap` — bit *i* set ⇔ `Authorities[i]` signed (≤ 64 authorities).
- `digest` — `blake2_256` over the concatenated signatures (a commitment).
- On finalise, the full `Attestations` list is pruned; only this 44-byte record
  remains. A finalised payload's footprint drops from `count × ~128` bytes to 44.

> **Not literal BLS.** The spec suggested BLS "to reduce storage"; there is no
> on-chain BLS aggregate-verify host function at this polkadot-sdk version, so
> true BLS would require pairing crypto inside the wasm runtime. This achieves
> the stated *goal* (storage reduction) with sr25519 retained. A BLS swap is a
> clean future change.

### 9.3 Inclusion commitment — ✅ live (via deferred finalisation)
`commit_inclusion(payload_id, deadline, signature)` records an authority's
signed promise (`InclusionCommitment`) to finalise a `PreConsensed` payload by
`deadline`. Breaking it (deadline passes, still not `Finalised`) is slashable
via `SlashProof::BrokenInclusion`.

Commitment message:
```
blake2_256(payload_id ++ "INCLUDE" ++ deadline.encode())
```

**The design tension it exposes — and how it is resolved.** A FRAME block
executes in a fixed order: `on_initialize` (all hooks) → inherents → signed
extrinsics → `on_finalize`. Auto-finalisation lives in `on_initialize`, so if a
payload were finalised *immediately* (the next block), it would be `Finalised`
before any `commit_inclusion` extrinsic in that block could run:

```
Block N:   attestations reach threshold  → PreConsensed
Block N+1: on_initialize finalises        → Finalised  (BEFORE extrinsics)
           commit_inclusion runs          → NotPreConsensed
```

§6.3 (immediate auto-finalise) and §14 (promise to include "in the next block")
are in direct tension there. **`FinalizeDelay` resolves it:** a payload that
reaches threshold at block `b` is scheduled to auto-finalise at `b +
FinalizeDelay` (see §9.1 / `FinalizeAt`). During those `FinalizeDelay` blocks it
sits in `PreConsensed`, so authorities have a real window to post commitments —
and finalisation is still automatic, just deferred. On the live chain
`FinalizeDelay = 2` (~12s at 6s/block), set in `spec_version: 103`.

**Why 2, and what §13 should say.** The original spec is self-contradictory
here: §13 states finalisation happens in the "next block", while §14 requires a
window *after* `PreConsensed` in which commitments can be posted. Both cannot
hold — the block-execution order above shows why. §14 wins, so **§13's "next
block" applies to `PreConsensed` (~1 block), not to `Finalised`.**

The delay was 10 blocks (~60s) through `spec_version: 102`. That was reduced
because it made the pallet the slowest clock in the system for no benefit:

| stage | mechanism | timing |
|---|---|---|
| `PreConsensed` | threshold signatures | ~1 block (~6s) |
| `Finalised` | `FinalizeAt[b + FinalizeDelay]` | 2 blocks (~12s) |
| GRANDPA finality | block consensus | ~3 blocks (~18s) |

`Finalised` is a **pallet state flag, not chain finality** (§1: "does NOT
replace block consensus or finality"). The block carrying the `PreConsensed`
event is canonically irreversible once GRANDPA finalises it at ~18s, regardless
of this pallet. Holding the flag for 60s therefore delayed a status update
*past* the point where the chain had already settled the matter. Keeping
`FinalizeDelay` under the GRANDPA lag restores the correct ordering: consensus,
not this pallet, is the binding constraint.

Note that nothing posts commitments automatically — the offchain worker submits
`attest_unsigned` only. `commit_inclusion` is reachable solely by manual
invocation (UI card, or `POST /commit-inclusion`). If an automatic producer is
ever added, revisit whether 2 blocks is a wide enough window for it.

A full live cycle — `submit → PreConsensed → commit_inclusion → finalize` — is
proven by `scripts/e2e-phase2.js`. Setting `FinalizeDelay = 0` restores
effectively-immediate finalisation (and closes the window again).

---

## 10. Runtime configuration

```rust
impl pallet_fastlane::Config for Runtime {
    type Currency = Balances;                 // ReservableCurrency for bonds
    type MinAuthorityBond = ConstU128<1_000>;
    type MaxPendingPayloads = ConstU32<100>;
    type MaxAuthorities = ConstU32<32>;
    type AuthorityId = pallet_fastlane::crypto::FastlaneAuthId; // "fast" key type
    type AllowedDomains = AllowedDomains;      // empty = all domains allowed
    type OffchainChecks = ();                  // default validation hook
    type MaxAutoFinalizePerBlock = ConstU32<50>;
    type FinalizeDelay = ConstU32<2>;          // ~12s PreConsensed window (§14)
    type MaxExpiriesPerBlock = ConstU32<...>;
    type OnFinalised = ();                     // downstream finalisation hook
    type WeightInfo = pallet_fastlane::weights::SubstrateWeight<Runtime>;
}
```

Security (§12) is enforced by: bonded, slashable, governance-rotated authorities;
`> 50%` threshold; bounded pending payloads and per-block work; expiry windows;
and the `PreConsensed`-is-not-final contract.

---

## 11. Deploying an upgrade

The pallet ships as part of the runtime wasm. To roll out changes to a live
chain **without losing history** (forkless upgrade):

1. Bump `spec_version` in `runtime/src/lib.rs` (and `transaction_version` if the
   extrinsic interface changed).
2. `cargo build --release -p solochain-template-runtime` →
   `target/release/wbuild/solochain-template-runtime/*.compact.compressed.wasm`.
3. `node scripts/upgrade-runtime.js ws://127.0.0.1:9944 <wasm>` — submits
   `sudo(system.setCode(...))` as Alice. Validators switch to the new on-chain
   wasm automatically; no restart needed.

`scripts/e2e-phase2.js` drives a live `submit → attest → aggregate →
commit_inclusion → finalize` check against a running node.
