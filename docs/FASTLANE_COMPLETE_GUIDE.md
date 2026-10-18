# FastLane Pre-Consensus Pallet: Complete Guide

What the spec says should happen, what the code in `/root/my-blockchain` actually does, and how to build, run, use, test and fix it.

> Everything below was read from the source files in this repo. File and line references are clickable in the IDE.
> Nothing in this guide changes code. Where I recommend a fix, it is marked **[RECOMMENDED CHANGE]**.

---

## Table of contents

1. [The 60-second version](#1-the-60-second-version)
2. [Glossary](#2-glossary)
3. [Repository map](#3-repository-map)
4. [The big picture](#4-the-big-picture)
5. [Spec requirement vs. reality (section by section)](#5-spec-requirement-vs-reality)
6. [Data model: storage, structs, events, errors](#6-data-model)
7. [Extrinsics, one by one](#7-extrinsics-one-by-one)
8. [The offchain worker in depth](#8-the-offchain-worker-in-depth)
9. [Unsigned transaction validation](#9-unsigned-transaction-validation)
10. [on_initialize: expiry and auto-finalise](#10-on_initialize)
11. [Cryptography and message formats](#11-cryptography-and-message-formats)
12. [The BLS aggregation path (spec section 14)](#12-the-bls-aggregation-path)
13. [Integrating other pallets (spec section 11)](#13-integrating-other-pallets)
14. [Governance, bonds and slashing](#14-governance-bonds-and-slashing)
15. [Runtime configuration values](#15-runtime-configuration-values)
16. [Block-by-block timeline of one payload](#16-block-by-block-timeline)
17. [How to run it](#17-how-to-run-it)
18. [How to use it (submit, watch, finalise)](#18-how-to-use-it)
19. [How to build this pallet from scratch](#19-how-to-build-this-pallet-from-scratch)
20. [Known problems and recommended fixes](#20-known-problems-and-recommended-fixes)
21. [Testing](#21-testing)
22. [Security and operations](#22-security-and-operations)
23. [Troubleshooting](#23-troubleshooting)
24. [Cheat sheet](#24-cheat-sheet)

---

## 1. The 60-second version

FastLane is a Substrate pallet that adds an **approval step before an action takes effect**.

1. A user calls `submit(...)`. The chain stores the **hash** of the action and marks it `Submitted`.
2. A committee of **authorities** (the bonded nodes) each check the payload.
3. Each authority node runs an **offchain worker (OCW)**. The OCW signs the payload and sends an **unsigned transaction**, `attest_unsigned`, to the chain.
4. When enough signatures are on-chain (`Threshold`, for example 3 of 5), the status becomes `PreConsensed`. A UI can show "approved".
5. Two blocks later (`FinalizeDelay = 2`), `on_initialize` flips it to `Finalised` and calls the downstream hook. Anyone can also call `finalize` earlier.
6. If the threshold is never reached before `expiry`, the status becomes `Expired`.

**Where the OCW lives:** its code is inside the on-chain runtime, but it **executes off-chain** on each node. It cannot change state directly. It can only submit transactions, and those are validated and executed on-chain like any other.

**The most important honest finding:** the spec says pre-consensus in under 1 second. This implementation needs **1–2 blocks (about 6–12 seconds)**, because an attestation only counts once it is included in a block. See [section 20](#20-known-problems-and-recommended-fixes).

---

## 2. Glossary

| Term | Meaning |
|---|---|
| **Pallet** | A Substrate module with its own storage, calls, events and errors. FastLane is one. |
| **Runtime** | The Wasm blob that contains all pallets and defines the chain's rules. Lives on-chain. |
| **Extrinsic** | An external call into the runtime (a "transaction"). Signed, unsigned, or inherent. |
| **Signed extrinsic** | Has a signature from an account that pays fees. |
| **Unsigned extrinsic** | No account, no fee. Only accepted if the pallet's `validate_unsigned` approves it. |
| **Inherent** | Data inserted by the block author (e.g. timestamp). Not what the OCW sends. |
| **Offchain worker (OCW)** | Runtime code that runs on a node after each block import, outside consensus. Result is non-deterministic and private to that node. |
| **Offchain storage** | Local per-node key/value DB. Not part of chain state. Not shared. |
| **Keystore** | Local per-node store of private keys. Files in `<base-path>/chains/<id>/keystore/`. |
| **Key type** | A 4-byte tag (`fast`, `aura`, `gran`) that says what a key may be used for. |
| **Authority** | An account in the `Authorities` list that is allowed to attest. |
| **Threshold** | Number of distinct authority attestations needed for `PreConsensed`. |
| **Bond** | Reserved balance an authority posts. It can be slashed. |
| **Payload** | The record of an action to approve. Only a hash of the data is stored. |
| **payload_id** | `blake2_256(SCALE(creator, nonce, domain, payload_hash))`. |
| **checks_hash** | A hash summarising the checks that passed. Authorities sign it, and the chain recomputes it. |
| **PreConsensed** | Threshold met. Fast approval. **Not** final. |
| **Finalised** | State transition applied. Canonical. |
| **GRANDPA** | The block finality gadget. Separate from FastLane finalisation. |
| **Aura** | The block production (slot) mechanism. |
| **BLS** | A signature scheme where many signatures combine into one. Used for the aggregation path. |
| **Sudo / root** | The privileged origin that calls `set_authorities`, `set_threshold`, `slash`. |

---

## 3. Repository map

```
/root/my-blockchain
├── Cargo.toml                    workspace manifest
├── FASTLANE_README.md            earlier README for the project
├── README.md                     upstream template README
├── ecosystem.config.js           pm2 definitions for the 5 validators
├── pallets/
│   ├── fastlane/                 <- THE PALLET
│   │   ├── src/lib.rs            2060 lines: everything
│   │   ├── src/mock.rs           mock runtime for unit tests
│   │   ├── src/tests.rs          unit tests
│   │   ├── src/bls_aggregation_tests.rs
│   │   ├── src/benchmarking.rs   weight benchmarks
│   │   ├── src/weights.rs        generated weight functions
│   │   └── examples/bls_gen.rs   helper to generate BLS keys
│   └── template/                 demo downstream pallet (FastLane-gated call)
├── runtime/
│   ├── src/lib.rs                construct_runtime, versions, offchain trait impls
│   ├── src/configs/mod.rs        every pallet's Config (FastLane values here)
│   ├── src/apis.rs               runtime APIs
│   └── src/genesis_config_presets.rs  initial authorities and threshold
├── node/
│   ├── src/service.rs            node service; starts the offchain worker runner
│   ├── src/chain_spec.rs         chain specs (dev, local)
│   └── src/command.rs            CLI (including `key insert`)
├── scripts/
│   ├── setup-keys.sh             put keystore keys on disk BEFORE starting
│   ├── run-dev.sh run-alice.sh run-bob.sh run-charlie.sh run-dave.sh run-eve.sh
│   ├── e2e-phase2.js             end-to-end test on a live chain
│   ├── bls-live-demo.js bls-ocw-demo.js upgrade-runtime.js
│   └── README.md                 deployment notes
├── docs/
│   ├── PALLET_REFERENCE.md
│   ├── OFFCHAIN_WORKER_REVIEW.md short review
│   └── FASTLANE_COMPLETE_GUIDE.md   this file
└── data/                         live node data (alice, bob, charlie, dave, eve)
```

Key places in `pallets/fastlane/src/lib.rs`:

| What | Lines |
|---|---|
| Downstream traits (`OnFinalised`, `EnsurePreConsensed`, `KycProvider`, `DomainRules`, `OffchainChecks`) | 84–200 |
| `Config` trait | 202–256 |
| Storage | 260–445 |
| Events | 454–490 |
| Errors | 496–540 |
| Data structures | 551–632 |
| Genesis | 634–672 |
| `on_initialize` | 684–774 |
| `offchain_worker` | 798–927 |
| `validate_unsigned` | 931–1075 |
| Extrinsics 0–12 | 1084–1545 |
| `do_attest` | 1550–1625 |
| BLS helpers | 1664–1945 |
| `validate_slash_proof` | 1954–2037 |
| `crypto` module (key type `fast`) | 2040–2060 |

---

## 4. The big picture

### 4.1 Three layers of "agreement"

```
Layer 1  FastLane pre-consensus   "the committee says OK"      ~1–2 blocks   NOT final
Layer 2  FastLane finalisation    "state transition applied"   +2 blocks     pallet-level
Layer 3  GRANDPA finality         "block can never revert"     ~3 blocks     chain-level
```

FastLane never replaces block consensus. A `PreConsensed` payload is **a statement signed by a committee**. If a block containing those attestations were reverted, so would the pre-consensus. That is why the spec says: *do not treat PreConsensed as irreversible*.

### 4.2 Who does what

```
 USER                      AUTHORITY NODE (x5)                    CHAIN (consensus)
  │                              │                                      │
  │ submit(nonce, expiry,        │                                      │
  │        domain, hash) ───────────────────────────────────────────►  │ store Payload, Status=Submitted
  │                              │                                      │ emit Submitted
  │                              │◄── block imported ───────────────────│
  │                              │ offchain_worker():                   │
  │                              │  - read Submitted payloads           │
  │                              │  - compute checks_hash               │
  │                              │  - sign                              │
  │                              │  - submit attest_unsigned ─────────► │ tx pool: validate_unsigned
  │                              │                                      │ next block: do_attest
  │                              │                                      │   count >= Threshold ?
  │                              │                                      │   -> PreConsensed
  │ UI shows "Approved" ◄────────────────────────────────────────────── │ emit PreConsensed
  │                              │                                      │ +2 blocks: on_initialize
  │ UI shows "Finalised" ◄───────────────────────────────────────────── │   -> Finalised, OnFinalised hook
```

### 4.3 Why an OCW at all

The chain cannot do network calls, read a KYC database, or take time to think. A node can. The OCW is the bridge that lets each authority apply its own judgement and then report the result back as a transaction.

### 4.4 Important consequence of how this is built

Because the chain **recomputes `checks_hash` itself** (see [section 8.7](#87-checks_hash-what-is-actually-checked)), the OCW cannot veto using any information the chain doesn't also have. All checks are effectively deterministic on-chain data. That is safe, but it is not the "each authority does its own private validation" the spec describes in section 7.

---

## 5. Spec requirement vs. reality

Legend: ✅ matches, ⚠️ partial or different, ❌ not met.

### Section 1: Objective

| Spec | Reality |
|---|---|
| Near-instant agreement on any payload | ⚠️ 1–2 blocks, not near-instant |
| Collect threshold signatures from a committee | ✅ `Attestations`, `Threshold`, `do_attest` |
| Mark payloads `PreConsensed` at threshold | ✅ [lib.rs:1603](../pallets/fastlane/src/lib.rs#L1603) |
| Finalise in the next block | ⚠️ finalises `FinalizeDelay` blocks later (configured as 2), or immediately via `finalize` |
| No actionable transition without pre-consensus | ✅ via `EnsurePreConsensed`, but only for pallets that opt in (see section 13) |
| Does not replace consensus | ✅ |

### Section 2: High-level architecture

| Spec | Reality |
|---|---|
| FastLane Authority Committee | ✅ `Authorities` storage, bonded, set by root |
| Flow Submitted → Attested → PreConsensed → Finalised | ✅ ("Attested" is not a stored status; it is the intermediate period when `Attestations` is partly filled) |

### Section 3: Lifecycle

```
Submitted ──threshold──► PreConsensed ──finalize/auto──► Finalised
    │
    └─ expiry passed ──► Expired
```

✅ Implemented exactly. `PreConsensed` never expires (the expiry check requires `Status::Submitted`, [lib.rs:696](../pallets/fastlane/src/lib.rs#L696)).

### Section 4: Storage layout

| Spec item | Implementation |
|---|---|
| `Authorities: BoundedVec<AuthorityId>` | ✅ `Authorities` (`BoundedVec<AccountId, MaxAuthorities>`) |
| `Threshold: u32` | ✅ |
| `AuthorityBonds: map` | ✅ |
| `Payloads: map` with creator, created_at, expiry, domain, payload_hash, status | ⚠️ `Payloads` holds all fields **except status**, plus `nonce`. Status is separate (below). |
| `Attestations: map PayloadId => BoundedVec<SignatureRecord>` | ✅ |
| `Status: map PayloadId => Status` | ✅ named `Statuses` |

Storage items the spec did not ask for, which the code added: `Nonces`, `PendingCount`, `LastSubmitAt`, `ExpiryAt`, `FinalizeAt`, `PreConsensedQueue` (legacy), `AggregatedAttestations`, `InclusionCommitments`, `AuthorityBlsKeys`, `BlsAttestations`, `BlsAggregate`.

### Section 5: Payload structure

| Spec | Implementation |
|---|---|
| `FastLanePayload { sender, nonce, domain, data_hash, expiry }` | Not a struct. `submit` takes `(nonce, expiry, domain, payload_hash)`; the sender is the signed origin. Equivalent. |
| Stores `data_hash`, not data | ✅ |
| `payload_id = blake2_256(sender + nonce + domain + data_hash)` | ✅ `blake2_256(SCALE-encode((who, nonce, domain, payload_hash)))` ([lib.rs:1132](../pallets/fastlane/src/lib.rs#L1132)). Note: SCALE encoding, not raw byte concatenation. Clients must build the same bytes. |

### Section 6: Extrinsics

| Spec | Implementation | Notes |
|---|---|---|
| 6.1 `submit`: expiry > now, nonce valid, domain allowed | ✅ plus KYC, cooldown, pending cap, duplicate check | |
| 6.2 `attest` as unsigned tx with signed payload | ✅ `attest_unsigned` (index 5). Signed `attest` (index 1) also exists | |
| 6.2 valid only if signer authority, status Submitted, not expired | ✅ `do_attest` | |
| 6.2 at threshold: PreConsensed, emit event | ✅ | |
| 6.3 `finalize` callable by anyone OR auto in `on_initialize` | ✅ both | |
| 6.4 `set_authorities`, `set_threshold`, `slash` | ✅ all root-only | Runtime uses sudo as root |

### Section 7: Offchain worker

| Spec | Reality |
|---|---|
| Each authority node runs an OCW | ✅ only if the node has a `fast` key in its keystore and `--offchain-worker always` |
| Monitor new Submitted events | ⚠️ it does not read events. It iterates `Statuses` storage each block |
| Validate: signature, nonce, balance, KYC, rate limit, domain rules | ⚠️ These are in `compute_checks_hash`, but KYC and domain rules are `()` (allow all) in the runtime and rate limit is checked at `submit`, not in the OCW (`rate_limit_ok = true`) |
| Sign `payload_id + expiry + checks_hash` | ✅ it signs `blake2_256` of that concatenation |
| Submit unsigned `attest(...)` | ✅ `attest_unsigned` |
| "Authorities must sign only if fully valid" | ✅ `compute_checks_hash` returns `None` if any check fails and the OCW skips the payload |

### Section 8: Threshold rules

| Spec | Reality |
|---|---|
| Threshold must be > 50% | ✅ in `set_threshold` ([lib.rs:1265](../pallets/fastlane/src/lib.rs#L1265)) |
| Never allow exactly 50% | ✅ `value > authority_count / 2` (integer division). For 5 authorities, `value > 2` means 3 or more. For 4 authorities, `value > 2` means 3 or more. Both correct |
| | ⚠️ Genesis does **not** run this check. A bad `threshold` in the chain spec would be accepted |

### Section 9: Expiry handling

| Spec | Reality |
|---|---|
| In `on_initialize`: `current > expiry` and `Submitted` → `Expired` | ✅ using the `ExpiryAt[expiry+1]` index, so no full scan |
| PreConsensed cannot expire | ✅ |
| (implicit) bounded work per block | ✅ `MaxExpiriesPerBlock`, leftovers pushed to the next block |

### Section 10: Slashing

| Spec | Reality |
|---|---|
| Slash for invalid payload signing | ⚠️ `SlashProof::InvalidSignature` exists but can practically never succeed, because invalid signatures are rejected before they are stored |
| Slash for double-signing | ❌ `SlashProof::DoubleSigning` only checks that the authority signed two **different** payloads. That is normal behaviour. See [section 20, issue 6](#issue-6-doublesigning-proof-is-meaningless) |
| Slash for broken inclusion commitment | ✅ `BrokenInclusion` |
| Proof submission, then governance OR automatic | ⚠️ governance (root) only. No automatic detection |
| Bonds in `AuthorityBonds` | ✅ plus actual reserved balance via `ReservableCurrency` |

### Section 11: Integration

| Spec | Reality |
|---|---|
| Downstream pallets verify status before acting | ✅ trait `EnsurePreConsensed`; `pallet-template` demonstrates it |
| FastLane as mandatory gateway | ⚠️ only mandatory for pallets that call the trait. Nothing stops a pallet from skipping it |

### Section 12: Security

| Spec | Reality |
|---|---|
| Expiry windows 10–30 blocks recommended | ⚠️ no upper bound enforced. `e2e-phase2.js` uses `now + 500` |
| Slashing | ⚠️ see above |
| Authority rotation via governance | ✅ `set_authorities` |
| Bond requirement | ✅ |
| Max pending payload limit | ✅ `MaxPendingPayloads = 100` |
| Don't allow threshold below majority | ✅ at `set_threshold`, ❌ at genesis |

### Section 13: Performance

| Spec | Reality |
|---|---|
| PreConsensus < 1 s | ❌ about 6–12 s |
| Finalisation next block | ⚠️ +2 blocks by default |
| Explorer shows both states | ⚠️ events and storage support it; the explorer lives in `fastlane-frontend` (not reviewed) |

### Section 14: Phase 2

| Spec | Reality |
|---|---|
| Inclusion commitment | ✅ `commit_inclusion` (manual only; no OCW posts them) |
| Auto-finalise batch in `on_initialize` | ✅ |
| BLS signature aggregation | ✅ real BLS12-381 via `w3f-bls`, aggregates stored in `BlsAggregate` |

---

## 6. Data model

### 6.1 Type aliases

```rust
pub type PayloadId = [u8; 32];
pub type BalanceOf<T> = <<T as Config>::Currency as Currency<AccountId>>::Balance;
```

### 6.2 Storage items

| Item | Type | Purpose | Written by | Read by |
|---|---|---|---|---|
| `Authorities` | `BoundedVec<AccountId, MaxAuthorities>` | Committee | genesis, `set_authorities`, `slash` (eviction) | OCW, `do_attest`, `validate_unsigned` |
| `Threshold` | `u32` | Signatures needed | genesis, `set_threshold` | `do_attest`, `do_attest_bls` |
| `AuthorityBonds` | `AccountId → Balance` | Bond ledger | genesis, `post_bond`, `slash` | `set_authorities`, `slash` |
| `Payloads` | `PayloadId → Payload` | Payload metadata | `submit` | everywhere |
| `Statuses` | `PayloadId → Status` | Lifecycle | `submit`, `do_attest`, `finalize`, hooks | everywhere |
| `Attestations` | `PayloadId → BoundedVec<SignatureRecord>` | sr25519 signatures | `do_attest` | OCW, `validate_unsigned`, slash |
| `Nonces` | `AccountId → u64` | Next expected nonce | `submit` | `submit`, `compute_checks_hash` |
| `PendingCount` | `u32` | Number of `Submitted` payloads | `submit`, `do_attest`, expiry | `submit` |
| `LastSubmitAt` | `AccountId → Block` | For cooldown | `submit` | `submit` |
| `ExpiryAt` | `Block → BoundedVec<PayloadId>` | Expiry schedule | `submit` | `on_initialize` |
| `FinalizeAt` | `Block → BoundedVec<PayloadId>` | Auto-finalise schedule | `do_attest` | `on_initialize` |
| `PreConsensedQueue` | `BoundedVec<PayloadId>` | Legacy queue, still drained | (nothing new writes it) | `on_initialize`, `finalize` |
| `AggregatedAttestations` | `PayloadId → AggregatedAttestation` | Permanent compact proof | `do_attest` | explorers |
| `InclusionCommitments` | `PayloadId → BoundedVec<InclusionCommitment>` | Promises | `commit_inclusion` | slash |
| `AuthorityBlsKeys` | `AccountId → bytes(≤128)` | BLS public keys | `register_bls_key*` | BLS attest |
| `BlsAttestations` | `PayloadId → Vec<(AccountId, sig≤64)>` | BLS signatures | `do_attest_bls` | OCW |
| `BlsAggregate` | `PayloadId → sig≤64` | One combined BLS signature | `do_attest_bls` | verifiers |

### 6.3 Structs

```rust
pub struct Payload<T> {
    pub creator: AccountId,
    pub created_at: BlockNumber,
    pub expiry: BlockNumber,
    pub domain: u32,
    pub payload_hash: [u8; 32],
    pub nonce: u64,
}

pub enum Status { Submitted, PreConsensed, Finalised, Expired }

pub struct SignatureRecord<T> {
    pub authority: AccountId,
    pub signature: Vec<u8>,      // 64 raw bytes (or SCALE MultiSignature)
    pub checks_hash: [u8; 32],
}

pub struct AggregatedAttestation {
    pub signer_bitmap: u64,      // bit i set => Authorities[i] signed
    pub digest: [u8; 32],        // blake2_256(concatenated signatures in authority order)
    pub count: u32,
}

pub struct InclusionCommitment<T> {
    pub authority: AccountId,
    pub committed_at: BlockNumber,
    pub deadline: BlockNumber,
    pub signature: Vec<u8>,
}

pub enum SlashProof {
    None,
    DoubleSigning(PayloadId, Vec<u8>, PayloadId, Vec<u8>),
    InvalidSignature(PayloadId, Vec<u8>),
    BrokenInclusion(PayloadId),
}
```

Note on `AggregatedAttestation.signer_bitmap`: it indexes the **current** `Authorities` ordering at the time the aggregate is built. If authorities are rotated later, old bitmaps no longer map to the same accounts. Keep that in mind in an explorer.

### 6.4 Events

| Event | Fields | When |
|---|---|---|
| `Submitted` | payload_id, creator | after `submit` |
| `PreConsensed` | payload_id | threshold met |
| `Finalised` | payload_id | `finalize` or auto |
| `Expired` | payload_id | expiry passed while `Submitted` |
| `AuthoritiesUpdated` | count | `set_authorities` |
| `ThresholdUpdated` | value | `set_threshold` |
| `AuthoritySlashed` | authority, amount | `slash` |
| `BondPosted` | authority, amount | `post_bond` |
| `AuthorityEvicted` | authority | slash left bond below minimum |
| `AttestationsAggregated` | payload_id, count | at threshold (compact record built) |
| `InclusionCommitted` | payload_id, authority, deadline | `commit_inclusion` |
| `BlsKeyRegistered` | authority | BLS key stored |
| `BlsAggregated` | payload_id, count | first BLS aggregate created |

### 6.5 Errors

| Error | Raised when |
|---|---|
| `PayloadNotFound` | unknown `payload_id` |
| `NotAuthority` | caller / signer not in `Authorities` |
| `AlreadyAttested` | duplicate sr25519 attestation, or attestation list full |
| `AlreadyPreConsensed` | attesting a payload already at threshold |
| `AlreadyFinalised` | finalising or attesting a finalised payload |
| `PayloadExpired` | `expiry <= now`, or status `Expired` |
| `ThresholdNotMet` | finalising a `Submitted` payload; gate check failure |
| `InvalidSignature` | bad sr25519 signature, bad `checks_hash`, decode failure |
| `ThresholdTooLow` | `set_threshold` ≤ 50% |
| `ThresholdTooHigh` | `set_threshold` > authority count |
| `DuplicatePayload` | same `payload_id` already exists |
| `InvalidNonce` | `nonce != Nonces[who]` |
| `DomainNotAllowed` | domain not in `AllowedDomains` (when non-empty) |
| `TooManyPendingPayloads` | `PendingCount` at cap, or expiry bucket full |
| `BondTooLow` | bond below `MinAuthorityBond` |
| `InsufficientBalance` | cannot reserve the bond |
| `SlashExceedsBond` | slash amount above bond |
| `MissingSlashProof` | proof is `None` or does not validate |
| `NotPreConsensed` | commitment on a payload not in `PreConsensed` |
| `AlreadyCommitted` | authority already committed |
| `InvalidDeadline` | deadline not in the future |
| `BlsKeyNotRegistered` | authority has no BLS key |
| `InvalidBlsPublicKey` | bytes do not parse |
| `InvalidBlsSignature` | BLS signature fails |
| `BlsAlreadyAttested` | duplicate BLS attestation |
| `NotKycVerified` | creator fails KYC |
| `SubmitTooFrequent` | cooldown not elapsed |

### 6.6 Genesis

[lib.rs:634](../pallets/fastlane/src/lib.rs#L634):

1. Convert `authorities` into a `BoundedVec` (panics if too many).
2. If `MinAuthorityBond` is non-zero, **reserve** that amount from each authority and record it in `AuthorityBonds`. The authority account must therefore already hold funds in the genesis balances.
3. Write `Authorities` and `Threshold`.
4. BLS keys are **not** set at genesis. Nodes register their own after start.

If an authority has no balance at genesis, `reserve` fails silently (`.is_ok()` is checked but the failure is ignored). That authority starts with **no bond** and `set_authorities` would later fail with `BondTooLow` for them.

---

## 7. Extrinsics, one by one

All indexes are the `#[pallet::call_index(n)]` values. Do not renumber them; clients depend on them.

### 7.0 Summary

| Idx | Name | Origin | Purpose |
|---|---|---|---|
| 0 | `submit` | signed | Register a payload |
| 1 | `attest` | signed (= authority) | Manual / test attestation |
| 2 | `finalize` | signed | Finalise a `PreConsensed` payload |
| 3 | `set_authorities` | root | Replace the committee |
| 4 | `set_threshold` | root | Change the threshold |
| 5 | `attest_unsigned` | none | OCW attestation |
| 6 | `post_bond` | signed | Reserve funds as bond |
| 7 | `slash` | root | Slash with proof |
| 8 | `commit_inclusion` | signed (authority) | Promise to include by a deadline |
| 9 | `register_bls_key` | signed (authority) | Register BLS key |
| 10 | `attest_bls` | signed (authority) | BLS attestation |
| 11 | `attest_bls_unsigned` | none | OCW BLS attestation |
| 12 | `register_bls_key_unsigned` | none | OCW self-registers BLS key |

### 7.1 `submit(nonce, expiry, domain, payload_hash)`

[lib.rs:1086](../pallets/fastlane/src/lib.rs#L1086). Checks run in this order:

1. Origin is signed → `who`.
2. `expiry > now`, else `PayloadExpired`.
3. `KycProvider::is_verified(who)`, else `NotKycVerified`.
4. If `SubmitCooldown != 0`: `now >= LastSubmitAt[who] + cooldown`, else `SubmitTooFrequent`.
5. `nonce == Nonces[who]`, else `InvalidNonce` (strictly sequential, starts at 0).
6. If `AllowedDomains` is non-empty: domain must be in it, else `DomainNotAllowed`.
7. `PendingCount < MaxPendingPayloads`, else `TooManyPendingPayloads`.
8. Compute `payload_id`; reject if it exists (`DuplicatePayload`).
9. Reserve a slot in `ExpiryAt[expiry+1]` (fails with `TooManyPendingPayloads` if that bucket is full).

Then it writes `Payloads`, `Statuses = Submitted`, `Nonces = nonce+1`, `LastSubmitAt`, `PendingCount += 1`, and emits `Submitted`.

Effects to remember:

- Nonce increments on **every** successful submit, even if the payload later expires. Nonces are strictly sequential, so one stuck/expired payload does not block the next.
- A single `expiry` bucket holds at most `MaxPendingPayloads` (100) ids, so a flood targeting one expiry block can fill it.
- No upper bound on `expiry`. A far-future expiry holds a pending slot for a long time.

### 7.2 `attest(payload_id, authority, signature, checks_hash)`

Signed version. `ensure!(who == authority)` then `do_attest`. Intended for tests and manual operation. Fees apply.

### 7.3 `finalize(payload_id)`

[lib.rs:1191](../pallets/fastlane/src/lib.rs#L1191). Any signed account.

- `Submitted` → `ThresholdNotMet`.
- `Finalised` → `AlreadyFinalised`.
- `Expired` → `PayloadExpired`.
- `PreConsensed` → remove from legacy queue, set `Finalised`, call `OnFinalised::on_finalised(payload_id)?` (if it errors, the whole extrinsic reverts and the status stays `PreConsensed`), prune signatures and commitments, emit `Finalised`.

Because `finalize` is a normal extrinsic, calling it early (before `FinalizeDelay`) bypasses the "inclusion commitment window". It's permissionless by design.

### 7.4 `set_authorities(new_authorities)`

Root only. Every new authority must have `AuthorityBonds >= MinAuthorityBond`. Replaces the list wholesale. Does **not** re-check the threshold. If you shrink the committee below the threshold, payloads can never reach pre-consensus until you also call `set_threshold`.

### 7.5 `set_threshold(value)`

Root only. Requires `authority_count/2 < value <= authority_count`.

| Authorities | Allowed thresholds |
|---|---|
| 3 | 2, 3 |
| 4 | 3, 4 |
| 5 | 3, 4, 5 |
| 7 | 4, 5, 6, 7 |
| 10 | 6–10 |
| 13 | 7–13 |

The spec's recommended 5/7, 7/10, 10/13 are all allowed.

### 7.6 `attest_unsigned(payload_id, authority_encoded, signature, checks_hash)`

`ensure_none(origin)`, decode authority, `do_attest`. Called only through the OCW path; pool validation is in [section 9](#9-unsigned-transaction-validation).

### 7.7 `post_bond(amount)`

Signed. `amount >= MinAuthorityBond` (1000), then `Currency::reserve`. Adds to `AuthorityBonds`. Any account can post a bond; being bonded does **not** make you an authority. Only `set_authorities` does that.

### 7.8 `slash(authority, amount, proof)`

Root only. [lib.rs:1334](../pallets/fastlane/src/lib.rs#L1334).

1. `proof != None`.
2. `amount <= AuthorityBonds[authority]`.
3. `validate_slash_proof` (see [section 14](#14-governance-bonds-and-slashing)).
4. `Currency::slash_reserved(authority, amount)` burns from the reserved balance.
5. Reduce the ledger, emit `AuthoritySlashed`.
6. If the remaining bond is below `MinAuthorityBond`, remove them from `Authorities` and emit `AuthorityEvicted`. The threshold is **not** adjusted automatically.

### 7.9 `commit_inclusion(payload_id, deadline, signature)`

Phase-2 feature. Caller must be an authority; payload must be `PreConsensed`; `deadline > now`; no earlier commitment by the caller; signature must verify over `blake2_256(payload_id ++ "INCLUDE" ++ deadline.encode())`. Nothing in the repo posts these automatically, so this is a manual operation.

### 7.10 `register_bls_key(public_key)` / `register_bls_key_unsigned(...)`

Signed form: caller is an authority and supplies a valid BLS public key. Unsigned form: the node proves ownership by signing `blake2_256("fastlane-bls-register" ++ public_key)` (see the helper) with its sr25519 authority key. See [section 12](#12-the-bls-aggregation-path).

### 7.11 `attest_bls(payload_id, signature)` / `attest_bls_unsigned(...)`

Verify a single BLS signature against the registered key, store it, and at threshold combine all into one aggregate. See [section 12](#12-the-bls-aggregation-path).

### 7.12 `do_attest` in detail

[lib.rs:1550](../pallets/fastlane/src/lib.rs#L1550). The core.

```
payload  = Payloads[id]            else PayloadNotFound
expiry   > now                     else PayloadExpired
status   == Submitted              else AlreadyPreConsensed / AlreadyFinalised / PayloadExpired
authority ∈ Authorities            else NotAuthority
authority ∉ Attestations[id]       else AlreadyAttested
checks_hash == compute_checks_hash(payload)   else InvalidSignature
msg_hash = blake2_256(id ++ expiry.encode() ++ checks_hash)
verify_authority_sig(authority, msg_hash, signature)   else InvalidSignature
push SignatureRecord, write Attestations[id]
if len >= Threshold:
    Statuses[id] = PreConsensed
    PendingCount -= 1
    FinalizeAt[now + FinalizeDelay].push(id)       (best effort)
    AggregatedAttestations[id] = build_aggregate(sigs)
    emit AttestationsAggregated, PreConsensed
```

Subtle points:

- `threshold > 0` is required for the transition. If `Threshold` is ever `0`, nothing can become `PreConsensed`.
- Once `PreConsensed`, further sr25519 attestations are rejected with `AlreadyPreConsensed`. So the signature set is **exactly the first `Threshold` signers**, not all authorities.
- If `FinalizeAt[...]` is full the payload is still `PreConsensed`, but will not auto-finalise. Someone must call `finalize` manually. The `let _ =` swallows this.
- Equal-block attestations: two attestations can land in the same block. The second one that would make `len >= Threshold` triggers the transition; any after that revert with `AlreadyPreConsensed` (and the pool's `Stale` check drops them when re-validated).

---

## 8. The offchain worker in depth

### 8.1 Is it on-chain or off-chain?

| Aspect | Answer |
|---|---|
| Where the code lives | In the runtime Wasm (on-chain code, upgradable with a runtime upgrade) |
| Where it executes | On each node's machine, in a separate task, **after the node imports a block** |
| Can it change chain state? | **No.** It can read state (at the imported block's state) and submit transactions |
| Is its output verified? | Only through the transactions it submits. They go through the normal pool + execution path |
| Is it deterministic / consensus-critical? | **No.** Each node may do different things, or nothing |
| Does it pay fees? | Not with unsigned transactions; validity is decided by `validate_unsigned` |

### 8.2 How the node starts it

[node/src/service.rs:187-204](../node/src/service.rs#L187):

```rust
if config.offchain_worker.enabled {
    let offchain_workers = sc_offchain::OffchainWorkers::new(OffchainWorkerOptions {
        runtime_api_provider: client.clone(),
        keystore: Some(keystore_container.keystore()),
        offchain_db: backend.offchain_storage(),
        transaction_pool: Some(OffchainTransactionPoolFactory::new(transaction_pool.clone())),
        network_provider: Arc::new(network.clone()),
        is_validator: role.is_authority(),
        enable_http_requests: true,
        custom_extensions: |_| vec![],
    });
    task_manager.spawn_handle().spawn(
        "offchain-workers-runner", "offchain-worker",
        offchain_workers.run(client.clone(), task_manager.spawn_handle()).boxed(),
    );
}
```

`config.offchain_worker.enabled` comes from the CLI flag **`--offchain-worker`**. The run scripts pass `--offchain-worker always`, which means "run it even on non-validator nodes". That is why Charlie, Dave and Eve (not block authors) also attest. The default is `when-authority`, which would skip them.

`--enable-offchain-indexing true` is separate. It lets the **runtime** write to offchain storage during block import. This pallet does not currently use indexing; it only reads/writes offchain-local storage from inside the OCW.

### 8.3 What triggers a run

The runner subscribes to block import notifications. For each **new best block** it calls the runtime API `OffchainWorkerApi::offchain_worker(header)`, which dispatches to every pallet's `offchain_worker(block_number)` hook. Notes:

- It runs on best-block imports, not on every block ever imported (during sync it is skipped).
- It is spawned as an async task and can overlap with the next block if it runs long.
- There is a time budget, so keep OCW work small.

### 8.4 The OCW line by line

[lib.rs:798](../pallets/fastlane/src/lib.rs#L798):

```rust
fn offchain_worker(block_number) {
```

**Step A: dedupe per block**

```rust
let store = StorageValueRef::persistent(b"fastlane::last_ocw_block");
if let Ok(Some(last)) = store.get::<BlockNumberFor<T>>() {
    if last >= block_number { return; }
}
store.set(&block_number);
```

Reads a local key. If this node already ran for this block number (or a later one), stop. This prevents double submission after a restart or re-org to the same height. Side effect: after a **re-org** to a lower block number, the worker is silent until the chain passes the old number again.

**Step B: find local keys**

```rust
let local_keys = sp_io::crypto::sr25519_public_keys(crate::crypto::KEY_TYPE); // "fast"
if local_keys.is_empty() { return; }
```

Lists every sr25519 public key in the node's keystore tagged `fast`. No key → nothing happens, silently. This is the usual reason "attestations never arrive" (documented in the pm2 file comment).

**Step C: make sure BLS key is registered**

For each local key whose account is in `Authorities`, call `ocw_ensure_bls_registered` (section 12).

**Step D: scan payloads**

```rust
for (payload_id, status) in Statuses::<T>::iter() {
    if status != Status::Submitted { continue; }
    ...
```

Iterates the **whole** `Statuses` map (see issue 4). For each `Submitted` payload:

1. `Payloads::get(id)`; skip if missing.
2. `checks_hash = T::OffchainChecks::compute_checks_hash(&payload)`; skip if `None` (a check failed).
3. If `Authorities` is empty, skip.
4. Build `msg = payload_id ++ payload.expiry.encode() ++ checks_hash` and `msg_hash = blake2_256(msg)`.
5. For each local key:
   - decode the 32-byte public key as `AccountId` (works because the runtime's `AccountId` is the raw sr25519 key);
   - skip if not in `Authorities`;
   - skip if it already appears in `Attestations[id]`;
   - `sr25519_sign(KEY_TYPE, key, &msg_hash)`;
   - build `Call::attest_unsigned { payload_id, authority_encoded, signature: 64 bytes, checks_hash }`;
   - `T::create_inherent(call.into())` builds the bare unsigned extrinsic;
   - `SubmitTransaction::submit_transaction(xt)` pushes it into the local transaction pool (result ignored with `let _`).
6. BLS part for the same key: derive the node's BLS seed, confirm the on-chain registered key matches, skip if already BLS-attested, sign, and submit `attest_bls_unsigned`.

### 8.5 The keys the OCW needs

| Key | Type | Where it lives | Created by | Used for |
|---|---|---|---|---|
| sr25519 authority key | key type `fast` | keystore | `setup-keys.sh` → `node key insert --key-type fast` | signing attestations; also the on-chain account identity |
| BLS secret seed | 32 random bytes | offchain-local storage key `fastlane::bls-seed::<account>` | generated on first use by `ocw_bls_seed` | signing BLS attestations |
| Aura key | sr25519, `aura` | keystore | `setup-keys.sh` | block production |
| GRANDPA key | ed25519, `gran` | keystore | `setup-keys.sh` | finality votes |

The `fast` key is the **same secret** as the validator's account key (the scripts insert the same SURI for all key types). That's convenient for development but means compromise of one compromises the other. A production design would use a separate `fast` key per authority and put its public key in `Authorities`.

### 8.6 Runtime plumbing the OCW needs

[runtime/src/lib.rs:243-280](../runtime/src/lib.rs#L243): the runtime implements `SigningTypes`, `CreateTransactionBase`, `CreateInherent` and `CreateSignedTransaction` for the FastLane `Call`. `CreateInherent::create_inherent` returns a **bare** (unsigned) extrinsic. In older Substrate this was `SendTransactionTypes` + `SubmitTransaction`. The name "inherent" is historical: it is not a block-author inherent.

`type AuthorityId = pallet_fastlane::crypto::FastlaneAuthId;` ([configs/mod.rs:185](../runtime/src/configs/mod.rs#L185)) ties the `fast` key type to `MultiSigner`/`MultiSignature`. The OCW as written does not use the `Signer` helper; it calls `sp_io::crypto::sr25519_sign` directly.

### 8.7 `checks_hash`: what is actually checked

[lib.rs:102](../pallets/fastlane/src/lib.rs#L102): the default `OffchainChecks for ()`:

| Check | Source | Effective today |
|---|---|---|
| `domain_ok` | `AllowedDomains` empty, or contains domain | depends on runtime config |
| `nonce_ok` | `Nonces[creator] >= payload.nonce + 1` | always true for a stored payload |
| `balance_ok` | creator's free balance is non-zero | real check |
| `kyc_ok` | `KycProvider::is_verified` | `()` → always true |
| `rate_limit_ok` | hard-coded `true` | no-op in OCW |
| `domain_specific_ok` | `DomainRules::is_allowed` | `()` → always true |

If all are true, it returns `blake2_256(SCALE(creator, nonce, domain, payload_hash, expiry, each bool))`. Otherwise `None`.

Consequences:

1. The OCW and the chain compute the same hash from the same on-chain data. The signed `checks_hash` therefore does not carry independent information. It is a deterministic function of chain state.
2. `balance_ok` is evaluated both when attesting and when applying `do_attest`. If the creator's balance becomes zero between those moments, `do_attest` fails with `InvalidSignature` (misleading error name).
3. To get true per-authority validation (HTTP KYC lookups, risk engines), `checks_hash` would need to be an **input** the chain cannot recompute, and the chain would need a policy (for example, "all signers must agree on the same hash"). That is a design change, see the recommended list.

### 8.8 What the OCW cannot do here

- It does not listen for `Submitted` events. It scans storage.
- It does not communicate with other authorities. There is no off-chain gossip of signatures.
- It does not retry. If the transaction is dropped from the pool, the next block's run re-checks `Attestations` and submits again.
- It does not post inclusion commitments.
- It does not handle expiry. `on_initialize` does that on-chain.

### 8.9 Why you cannot get under one second this way

```
t=0.0  user's submit tx lands in block N                   (block time 6 s)
t=6.0  block N+1 imported by authority nodes -> OCW runs   (<= 6 s wait)
t=6.x  attest_unsigned enters the pool
t=12   block N+2 includes the attestations (<= 6 s wait)
       threshold met -> PreConsensed
```

Expected user wait: **6–12 s**. To go below one second the authorities must exchange signatures over a gossip channel (libp2p request-response or a dedicated RPC) and a client (or aggregator) must collect `Threshold` signatures **before** a block exists. The PreConsensed flag would then be stored on-chain only afterwards. That is a different architecture (see section 20).

---

## 9. Unsigned transaction validation

Unsigned extrinsics bypass fee and signature checks, so the pool asks the pallet: *"is this one legitimate?"* via `ValidateUnsigned`. [lib.rs:931](../pallets/fastlane/src/lib.rs#L931).

### 9.1 For `attest_unsigned`

Checks in order:

1. `Statuses[id]` exists and equals `Submitted`, else `Stale`.
2. `Payloads[id]` exists and `now <= expiry`, else `Stale`.
3. `authority_encoded` decodes to an `AccountId`, else `BadProof`.
4. Authority ∈ `Authorities`, else `BadProof`.
5. Authority not already in `Attestations[id]`, else `Stale`.
6. `checks_hash == compute_checks_hash(payload)`, else `BadProof`.
7. `signature` non-empty, else `BadProof`.

Then it returns `ValidTransaction` with:

- `tag_prefix("FastLaneAttest")`
- `priority(100)`
- `longevity(5)` blocks
- `provides((payload_id, authority_encoded))` → the **deduplication tag**: only one such tx per (payload, authority) in the pool
- `propagate(true)` → gossiped to peers

**Gap: step 7 does not verify the signature.** See [issue 3](#issue-3-attest_unsigned-signature-is-not-verified-in-the-pool).

### 9.2 For `attest_bls_unsigned`

Same shape, but it **does verify** the BLS signature against the registered key (`bls_verify_single`) before accepting.

### 9.3 For `register_bls_key_unsigned`

Checks the authority is in the set, the key parses, the key isn't already registered with the same bytes, and the sr25519 `proof` verifies via `verify_authority_sig`.

### 9.4 Why `provides` matters

Without a `provides` tag, ten copies of the same attestation could fill the pool. With the tag, the second is rejected as already known. It also means that a forged transaction that passes `validate_unsigned` can **take the slot** of the real one until it is mined and rejected.

---

## 10. on_initialize

[lib.rs:684](../pallets/fastlane/src/lib.rs#L684). Runs at the **start of each block**, before extrinsics. Cost is bounded.

### 10.1 Expiry

```
scheduled = ExpiryAt.take(now)              // payloads whose expiry+1 == now
for each id (up to MaxExpiriesPerBlock):
    if Statuses[id] == Submitted:
        Statuses[id] = Expired
        PendingCount -= 1
        emit Expired
leftovers -> ExpiryAt[now+1]  (best effort)
```

The bucket key is `expiry + 1`, so `now > expiry` is exactly the condition. `PreConsensed` or `Finalised` entries in the bucket are ignored (stay as they are).

### 10.2 Auto-finalise

Two sources share a per-block budget `MaxAutoFinalizePerBlock`:

1. **Legacy `PreConsensedQueue`**: drains up to the budget.
2. **`FinalizeAt[now]`**: drains until the budget is hit, pushes overflow to `now+1`.

For each id, if `Statuses == PreConsensed` and `OnFinalised::on_finalised(id).is_ok()`: set `Finalised`, prune, emit `Finalised`.

**Gap:** if `on_finalised` returns an error, nothing re-schedules the payload. It stays `PreConsensed` forever unless someone calls `finalize` manually (which would fail the same way). See [issue 7](#issue-7-failed-downstream-hook-strands-the-payload).

### 10.3 Weight

Returns `on_initialize_expire(expired_count) + on_initialize_finalize(finalized_count)` from the benchmarked weights.

---

## 11. Cryptography and message formats

### 11.1 The sr25519 attestation message

```
msg      = payload_id (32 bytes)
         ++ expiry.encode()           // SCALE of BlockNumber (u32 → 4 bytes little-endian)
         ++ checks_hash (32 bytes)
msg_hash = blake2_256(msg)            // 32 bytes
signature = sr25519_sign(key, msg_hash)    // 64 bytes
```

The spec says `sign(payload_id + expiry + checks_hash)`. The implementation signs the **hash** of that, which is fine and cleaner for sr25519.

`BlockNumber` is `u32` in this runtime, so `expiry.encode()` is 4 bytes. If you change `BlockNumber` to `u64`, every client and signature changes.

### 11.2 Signature verification

`verify_authority_sig` accepts:

- a raw 64-byte sr25519 signature, or
- a SCALE `MultiSignature::Sr25519(..)`.

`AccountId` must encode to exactly 32 bytes (it is the sr25519 public key). `sp_io::crypto::sr25519_verify` does the check.

### 11.3 Inclusion message

```
blake2_256( payload_id ++ b"INCLUDE" ++ deadline.encode() )
```

### 11.4 BLS message

`Message::new(b"fastlane-attest", payload_id)`. All authorities sign the **same message**, which is what allows aggregation by simple addition.

### 11.5 BLS registration proof message

`bls_register_message(public_key)` hashes a domain-separated string with the BLS public key; the authority signs that hash with sr25519 to prove they own the BLS key.

### 11.6 Computing `payload_id` in a client

```js
const { blake2AsU8a } = require('@polkadot/util-crypto');
const pid = u8aToHex(blake2AsU8a(
  api.registry.createType('(AccountId, u64, u32, [u8;32])',
                          [creator, nonce, domain, payloadHash]).toU8a(), 256));
```

This is exactly what [scripts/e2e-phase2.js](../scripts/e2e-phase2.js) does. Do **not** concatenate raw bytes: `nonce` is SCALE `u64` little-endian, `domain` is `u32` little-endian.

### 11.7 Replay protection

- `payload_id` includes `creator`, `nonce`, `domain`, `payload_hash`. Same data, different nonce → different id.
- `Nonces[who]` must match exactly, so a replay of an old `submit` fails with `InvalidNonce`.
- Attestations bind to `payload_id`, `expiry` and `checks_hash`. They can't be reused for another payload.
- There is no chain-id in the signed message. The same authority key on two chains **with the same payload_id** could replay an attestation. Add a genesis hash into the signed bytes if you run multiple chains with shared keys.

---

## 12. The BLS aggregation path

Spec section 14 says "Signature aggregation (BLS) to reduce storage." Two layers exist.

### 12.1 Layer A: sr25519 compaction (`AggregatedAttestations`)

When the threshold is reached, `build_aggregate` produces:

- `signer_bitmap` (u64): bit `i` set if `Authorities[i]` signed
- `digest`: `blake2_256` of the concatenated signatures in authority order
- `count`

After finalisation `prune_finalised_storage` deletes the full `Attestations` list and `InclusionCommitments`, leaving only the compact record. This reduces storage but is **not** cryptographic aggregation; the digest only commits to signatures that are now deleted.

### 12.2 Layer B: real BLS12-381 aggregation

Uses the `w3f-bls` crate, `TinyBLS381` engine (public keys ≈ 96 bytes, signatures ≈ 48 bytes). All inside the runtime Wasm; no new host functions.

**Registration** (once per authority):

1. First OCW run: `ocw_bls_seed(auth)` creates 32 random bytes and stores them in offchain storage under `fastlane::bls-seed::<account>`.
2. `bls_public_from_seed` derives the BLS public key.
3. If on-chain `AuthorityBlsKeys[auth]` differs, sign the registration message with the sr25519 key and submit `register_bls_key_unsigned`.

**Attestation** (per payload):

1. OCW signs `Message(b"fastlane-attest", payload_id)` with the BLS secret.
2. Submits `attest_bls_unsigned`. The pool verifies the signature (`bls_verify_single`).
3. `do_attest_bls` stores `(authority, sig)` in `BlsAttestations`.
4. At `len >= Threshold`, `aggregate_bls_signatures` sums the signatures into one and stores it in `BlsAggregate`. The aggregate is refreshed when more signatures arrive.

**Verification** (anyone): `verify_bls_aggregate(ctx, payload_id, pubkeys, aggregate_sig)` sums the public keys and verifies the one signature.

### 12.3 Properties and limits

| Property | Value |
|---|---|
| Storage per payload after aggregation | one ~48-byte aggregate (plus raw `BlsAttestations` until pruned) |
| Does BLS affect `PreConsensed`? | **No.** `PreConsensed` is decided by the sr25519 path only. BLS is parallel evidence |
| Are `BlsAttestations` pruned? | **No.** `prune_finalised_storage` removes sr25519 data only |
| Rogue-key attacks | Registration is authenticated by an sr25519 proof from the authority, which defuses the usual rogue-key problem since keys can't be chosen adversarially without the authority's signature. A proof-of-possession of the BLS secret itself is not checked |
| BLS attestation requires `Submitted`? | No. It works until expiry, independent of sr25519 progress |

---

## 13. Integrating other pallets

### 13.1 The gate trait

```rust
pub trait EnsurePreConsensed {
    fn ensure_preconsensed(payload_id: PayloadId) -> DispatchResult;
}
impl<T: Config> EnsurePreConsensed for Pallet<T> { ... }
```

Passes if the status is `PreConsensed` **or** `Finalised`; fails with `PayloadNotFound` or `ThresholdNotMet` otherwise. It is deliberately not generic over FastLane's `Config` so other pallets don't depend on it.

### 13.2 The finalisation hook

```rust
pub trait OnFinalised { fn on_finalised(payload_id: PayloadId) -> DispatchResult; }
```

In the runtime today: `type OnFinalised = ();` which does nothing. A real consumer would implement it to apply the state change.

### 13.3 The demo: `pallet-template`

[pallets/template/src/lib.rs](../pallets/template/src/lib.rs):

```rust
pub trait Config: frame_system::Config {
    type PreConsensusGate: EnsurePreConsensed;
    ...
}

#[pallet::call_index(2)]
pub fn do_something_gated(origin, payload_id, something: u32) -> DispatchResult {
    let who = ensure_signed(origin)?;
    T::PreConsensusGate::ensure_preconsensed(payload_id)?;   // <- the gate
    Something::<T>::put(something);
    ...
}
```

In the runtime: `type PreConsensusGate = Fastlane;` ([configs/mod.rs:174](../runtime/src/configs/mod.rs#L174)).

### 13.4 What the gate does **not** check

`ensure_preconsensed(payload_id)` only confirms that **a** payload with that id is approved. It does **not** verify:

- that the caller is the payload's creator,
- that the call's arguments match the payload's `payload_hash`,
- that the payload has not already been consumed (it can be used by many gated calls).

For a real integration, the downstream pallet should:

1. load `Payloads[payload_id]`,
2. require `creator == who`,
3. recompute `blake2_256(SCALE(call_data))` and require it equals `payload.payload_hash`,
4. record the id as consumed to prevent reuse.

Without these, the demo gated call is effectively "any approved payload id lets anyone set anything." **[RECOMMENDED CHANGE]** for production use.

### 13.5 Wiring a new pallet

1. Add `pallet-fastlane` as a dependency.
2. Add `type PreConsensusGate: pallet_fastlane::EnsurePreConsensed;` to your `Config`.
3. Call `T::PreConsensusGate::ensure_preconsensed(id)?;` first in each gated call.
4. In `runtime/src/configs/mod.rs`: `type PreConsensusGate = Fastlane;`.
5. For finalisation-driven logic, implement `OnFinalised` and set `type OnFinalised = YourPallet;` in the FastLane config.

---

## 14. Governance, bonds and slashing

### 14.1 Who is "root"

The runtime includes `pallet-sudo`. Whoever holds the sudo key can call `set_authorities`, `set_threshold` and `slash` via `sudo.sudo(call)`. The scripts' README warns to replace sudo with real governance before mainnet. The spec's "slashing by governance" is therefore a **single key** today.

### 14.2 Bond lifecycle

```
account ──post_bond(≥1000)──► reserved balance + AuthorityBonds[account]
root ──set_authorities([..])──► only accounts with bond ≥ MinAuthorityBond may be included
root ──slash(acct, amt, proof)──► burn from reserved balance, reduce ledger
                                   if ledger < MinAuthorityBond → auto-evicted
```

There is **no unbond / withdraw extrinsic**. Once an account posts a bond, the funds stay reserved indefinitely; there is no exit path. **[RECOMMENDED CHANGE]** add `unbond` with a delay.

### 14.3 Slash proofs

| Proof | Validation | Realistic? |
|---|---|---|
| `None` | rejected | n/a |
| `InvalidSignature(id, sig)` | finds the authority's stored record with exactly `sig`, rebuilds the message, requires verification to **fail** | Practically unreachable: `do_attest` only stores signatures that verify, so no failing record can exist |
| `DoubleSigning(idA, sigA, idB, sigB)` | `idA != idB` and both signatures are present in `Attestations` for that authority | Meaningless (issue 6): attesting two payloads is normal |
| `BrokenInclusion(id)` | the authority has a commitment on `id`, its deadline passed, payload not `Finalised` | Works. Note: commitments are deleted when the payload finalises, which is why the "not finalised" guard rarely matters |

### 14.4 What triggers eviction

Only a slash that leaves `bond < MinAuthorityBond`. `slash` with a partial amount that keeps the bond above the minimum leaves the authority in place.

### 14.5 After eviction

`Authorities` shrinks. `Threshold` is unchanged. If `authorities.len() < Threshold`, the chain can no longer pre-consense new payloads. Fix with `set_authorities` and/or `set_threshold` via root.

---

## 15. Runtime configuration values

[runtime/src/configs/mod.rs:176](../runtime/src/configs/mod.rs#L176):

| Config | Value | Meaning |
|---|---|---|
| `Currency` | `Balances` | bonds use reserved balance |
| `MinAuthorityBond` | 1,000 | minimum bond (smallest units) |
| `MaxPendingPayloads` | 100 | cap on `Submitted` payloads; also bucket sizes |
| `MaxAuthorities` | 50 | committee size cap (the Aura/GRANDPA `MaxAuthorities` is a separate 32) |
| `AuthorityId` | `FastlaneAuthId` | key type `fast` |
| `AllowedDomains` | empty `Vec` | empty = all domains allowed |
| `OffchainChecks` | `()` | default checks |
| `MaxAutoFinalizePerBlock` | 50 | auto-finalise budget per block |
| `FinalizeDelay` | 2 blocks | `PreConsensed` → auto `Finalised` |
| `MaxExpiriesPerBlock` | 50 | expiries processed per block |
| `KycProvider` | `()` | everyone passes |
| `DomainRules` | `()` | everything allowed |
| `SubmitCooldown` | 0 | rate limit off |
| `OnFinalised` | `()` | no downstream action |
| `WeightInfo` | `SubstrateWeight<Runtime>` | benchmarked weights |

Runtime versioning: `runtime/src/lib.rs` comments track `spec_version` bumps (for example 103→104 for §11 integration) and a transaction-version history (call indexes added: BLS ones as 9–12). Any change to a call's signature or index requires a `transaction_version` bump; storage-incompatible changes need a migration.

---

## 16. Block-by-block timeline

Assume 6 s blocks, the live committee of 4 authorities (Alice, Bob, Charlie, Dave), `Threshold = 3`, `FinalizeDelay = 2`, creator = Alice, nonce 0, `expiry = 100`.

```
Block 50
  Extrinsic: Fastlane.submit(0, 100, 1, 0xabc…)
  State:   Payloads[id], Statuses[id]=Submitted, Nonces[Alice]=1, PendingCount+1,
           ExpiryAt[101] += id
  Event:   Submitted{id, Alice}

(block 50 imported by all 5 nodes; Eve is not an authority, so her OCW finds no matching key in `Authorities` and submits nothing)
  Each node's OCW(50):
     last_ocw_block < 50 → proceed, set to 50
     finds its `fast` key → authority? yes
     ensure BLS key registered (first time: submits register_bls_key_unsigned)
     Statuses.iter(): finds `id` Submitted
     checks_hash ok → sign → submit attest_unsigned to local pool
     (also attest_bls_unsigned if BLS key already registered)
  Txs propagate through the network pools.

Block 51  (author = e.g. Bob)
  Pool contents: 4 attest_unsigned (one per authority) + BLS ones
  Block includes them (order by priority/arrival).
  do_attest #1 (Alice)   → Attestations: 1/3
  do_attest #2 (Bob)     → 2/3
  do_attest #3 (Charlie) → 3/3 ✔ threshold
       Statuses[id]=PreConsensed, PendingCount-1,
       FinalizeAt[53].push(id),
       AggregatedAttestations[id] = {bitmap, digest, count=3}
       Events: AttestationsAggregated, PreConsensed
  do_attest #4 (Dave)    → fails AlreadyPreConsensed (the later attestation is rejected)

  UI: "Approved (PreConsensed)"  ← about 6–12 s after submit

Block 52
  nothing for this payload

Block 53
  on_initialize: FinalizeAt.take(53) = [id]
     Statuses==PreConsensed → OnFinalised::on_finalised(id) → Ok
     Statuses[id]=Finalised, prune Attestations, emit Finalised
  UI: "Finalised"

~Block 54–56
  GRANDPA finalises block 51 (canonical finality of the pre-consensus)
```

Alternative branch, no threshold:

```
Block 101
  on_initialize: ExpiryAt.take(101) = [id]
  Statuses==Submitted → Expired, PendingCount-1, event Expired
```

---

## 17. How to run it

### 17.0 What is running on this server right now

Read from the live chain on 2026-10-09 (RPC `127.0.0.1:9944`):

| Item | Value |
|---|---|
| Runtime `spec_version` / `transaction_version` | 112 / 6 |
| Best block | ~775,800 |
| `Authorities` | Alice, Bob, Charlie, Dave (4) |
| `Threshold` | 3 (of 4, which is 75%, > 50% ✔) |
| `PendingCount` | 0 |
| Payloads stored | 26, **all `Finalised`** |
| Nodes (pm2) | `fastlane-alice` 9944, `-bob` 9945, `-charlie` 9946, `-dave` 9947, `-eve` 9948 |
| Block authors (`--validator`) | Alice, Bob |
| Eve | runs an OCW, but is **not** in `Authorities`, so it attests nothing |

Note the difference from the `local` genesis preset (Alice + Bob, threshold 2): the live chain was changed after genesis with `set_authorities` and `set_threshold`.

Another chain, `tenderchain`, runs separately on RPC 9955 (`--dev`). It is unrelated to this one.

### 17.1 Prerequisites

- Rust toolchain from `rust-toolchain.toml` / [docs/rust-setup.md](rust-setup.md).
- On this box (GCC 15), any RocksDB-using Rust build after a `cargo clean` needs:
  `export CXXFLAGS="-include cstdint"`.
- `clang`, `libclang-dev`, `protobuf-compiler`, `pkg-config`, `libssl-dev`.
- Node.js + `@polkadot/api` for the JS scripts (installed under `/root/fastlane-api/node_modules`).

### 17.2 Build

```bash
cd /root/my-blockchain
export CXXFLAGS="-include cstdint"      # needed here after a clean build
cargo build --release                    # produces target/release/solochain-template-node
```

The runtime Wasm is built by `runtime/build.rs` (substrate-wasm-builder) and embedded in the node. Build logs from earlier runs: `/root/my-blockchain-build*.log`.

Run the pallet tests without building the node:

```bash
cargo test -p pallet-fastlane
cargo test -p pallet-template
```

### 17.3 Single-node development chain (1 authority, threshold 1)

```bash
./scripts/setup-keys.sh ./data/dev dev //Alice      # keystore: fast + aura + gran + node key
./scripts/run-dev.sh
```

`--chain dev` uses the preset with `Alice` as the only FastLane authority and `threshold = 1`. Connect Polkadot.js Apps to `ws://127.0.0.1:9944`.

### 17.4 Multi-validator local network

One keystore per node, each with its own key:

```bash
./scripts/setup-keys.sh ./data/alice   local //Alice
./scripts/setup-keys.sh ./data/bob     local //Bob
./scripts/setup-keys.sh ./data/charlie local //Charlie
./scripts/setup-keys.sh ./data/dave    local //Dave
./scripts/setup-keys.sh ./data/eve     local //Eve

./scripts/run-alice.sh      # start first: bootnode with fixed node-key ...0001
./scripts/run-bob.sh
./scripts/run-charlie.sh
./scripts/run-dave.sh
./scripts/run-eve.sh
```

Or under pm2 (as on this server):

```bash
pm2 start /root/my-blockchain/ecosystem.config.js
pm2 save
pm2 logs fastlane-alice
```

What `setup-keys.sh` does for each node (SURI via argument, `$FASTLANE_SURI` or stdin prompt):

| Command | Effect |
|---|---|
| `key insert --key-type fast --scheme sr25519` | the OCW attestation key |
| `key insert --key-type aura --scheme sr25519` | block production |
| `key insert --key-type gran --scheme ed25519` | GRANDPA |
| `key generate-node-key` | libp2p peer identity |

Important CLI flags used by the run scripts:

| Flag | Why |
|---|---|
| `--offchain-worker always` | run the OCW on every node, not only block authors |
| `--enable-offchain-indexing true` | allow runtime offchain indexing (not used by FastLane itself) |
| `--validator` | author blocks (Alice and Bob only here) |
| `--rpc-methods safe` | blocks `author_insertKey` etc. |
| `--unsafe-rpc-external` | **binds RPC to all interfaces**, see [section 22](#22-security-and-operations) |
| `--bootnodes /ip4/127.0.0.1/tcp/30333/p2p/<AlicePeerId>` | peers dial Alice |

### 17.5 Becoming an authority: the full procedure

An account must (1) hold funds, (2) post a bond, (3) be added by root, (4) have its `fast` key on a node.

```text
1. Fund the account            balances.transfer
2. Bond                        fastlane.postBond(amount >= 1000)
3. Insert the key on a node    ./scripts/setup-keys.sh <base-path> <chain> "<SURI>"
                               then restart that node with --offchain-worker always
4. Root adds it                sudo.sudo(fastlane.setAuthorities([...existing, new]))
5. Adjust threshold            sudo.sudo(fastlane.setThreshold(n))   // > 50% of the new size
6. Wait ~2 blocks              the node's OCW registers its BLS key automatically
```

Check:

```text
fastlane.authorities()           -> includes the account
fastlane.authorityBlsKey(acct)   -> Some(<bytes>)   (after the OCW ran once)
fastlane.authorityBonds(acct)    -> >= 1000
```

### 17.6 Removing or rotating authorities

```text
sudo.sudo(fastlane.setAuthorities([...new list...]))   // each must have bond >= MinAuthorityBond
sudo.sudo(fastlane.setThreshold(n))                    // keep > 50% and <= len
```

Order matters if you shrink the list: if `len < Threshold` payloads stall. Lower the threshold **first** (`set_threshold` checks against the **current** authority count), then shrink, then re-check.

There is no way to withdraw a bond (section 14.2).

### 17.7 Upgrading the runtime

Changing the pallet:

1. Edit code, bump `spec_version` (and `transaction_version` if calls changed) in `runtime/src/lib.rs`.
2. If storage layout changed, add a migration. Keep `#[pallet::call_index]` values stable.
3. `cargo build --release` produces `target/release/wbuild/.../*.compact.compressed.wasm`.
4. Submit with `sudo.sudoUncheckedWeight(system.setCode(wasm), weight)` (see [scripts/upgrade-runtime.js](../scripts/upgrade-runtime.js)).
5. Nodes pick up the new runtime at the next block. **No node-binary rebuild is needed for runtime-only changes**, including OCW logic, because the OCW is runtime code.

The BLS work was done as forkless upgrades this way (runtime comment: "activating real BLS is a forkless runtime upgrade").

---

## 18. How to use it

### 18.1 Submit a payload (Polkadot.js Apps UI)

1. Developer → Extrinsics → `fastlane` → `submit`.
2. `nonce`: `fastlane.nonces(<your account>)` (starts at 0, +1 per successful submit).
3. `expiry`: current block + 10 to 30 recommended.
4. `domain`: any `u32` (all allowed here).
5. `payloadHash`: 32 bytes, for example `blake2_256` of your off-chain data.
6. Submit and watch Developer → Chain state → `fastlane.statuses(<id>)`.

### 18.2 Submit and wait with a script

```js
const { ApiPromise, WsProvider } = require('@polkadot/api');
const { Keyring } = require('@polkadot/keyring');
const { cryptoWaitReady, blake2AsU8a } = require('@polkadot/util-crypto');
const { u8aToHex, stringToU8a } = require('@polkadot/util');

(async () => {
  await cryptoWaitReady();
  const api = await ApiPromise.create({ provider: new WsProvider('ws://127.0.0.1:9944') });
  const alice = new Keyring({ type: 'sr25519' }).addFromUri('//Alice');

  const now    = (await api.query.system.number()).toNumber();
  const nonce  = (await api.query.fastlane.nonces(alice.address)).toNumber();
  const domain = 1;
  const expiry = now + 20;
  const hash   = u8aToHex(blake2AsU8a(stringToU8a('my data'), 256));

  const pid = u8aToHex(blake2AsU8a(
    api.registry.createType('(AccountId, u64, u32, [u8;32])',
                            [alice.address, nonce, domain, hash]).toU8a(), 256));

  await api.tx.fastlane.submit(nonce, expiry, domain, hash).signAndSend(alice);

  for (let i = 0; i < 20; i++) {
    await new Promise(r => setTimeout(r, 3000));
    const s = (await api.query.fastlane.statuses(pid)).toString();
    console.log(i, s);
    if (s === 'Finalised' || s === 'Expired') break;
  }
  await api.disconnect();
})();
```

Expected output with healthy authorities: `Submitted` for 1–2 polls, `PreConsensed` for ~1–2 polls (about 12 s), then `Finalised`.

### 18.3 Subscribe to events instead of polling

```js
api.query.system.events((events) => {
  events.forEach(({ event }) => {
    if (event.section === 'fastlane') {
      console.log(event.method, event.data.toHuman());
    }
  });
});
```

Events to show in a UI: `Submitted` → "Submitted", `PreConsensed` → "Approved", `Finalised` → "Finalised", `Expired` → "Expired".

### 18.4 Run the shipped end-to-end test

```bash
cd /root/fastlane-api            # has @polkadot/api installed
node /root/my-blockchain/scripts/e2e-phase2.js ws://127.0.0.1:9944
```

It does: submit → wait for OCW attestations and the aggregate → `commit_inclusion` → `finalize` → confirms signatures were pruned and the aggregate retained, then prints `E2E PASS`. It signs as `//Alice`, so Alice must be an authority. If it hangs on "waiting for aggregate", the OCWs are not attesting (see [section 23](#23-troubleshooting)).

### 18.5 Use the gate from another pallet

```js
// 1) payload must be PreConsensed (or Finalised)
await api.tx.template.doSomethingGated(pid, 42).signAndSend(alice);
```

Before approval the call fails with `fastlane.ThresholdNotMet`; with an unknown id, `fastlane.PayloadNotFound`. (Check the exact pallet name in the runtime metadata: `api.tx` keys.)

### 18.6 Governance actions via sudo

```js
const sudo = new Keyring({ type: 'sr25519' }).addFromUri('//Alice');   // whoever holds the sudo key
await api.tx.sudo.sudo(api.tx.fastlane.setThreshold(3)).signAndSend(sudo);
await api.tx.sudo.sudo(api.tx.fastlane.setAuthorities([a, b, c, d])).signAndSend(sudo);
await api.tx.sudo.sudo(api.tx.fastlane.slash(addr, 500, { BrokenInclusion: pid })).signAndSend(sudo);
```

### 18.7 Useful read-only queries

| Question | Query |
|---|---|
| Who is on the committee? | `fastlane.authorities()` |
| How many signatures needed? | `fastlane.threshold()` |
| Status of a payload | `fastlane.statuses(id)` |
| Who has signed so far? | `fastlane.attestations(id)` |
| Compact proof after finalise | `fastlane.aggregatedAttestations(id)` |
| BLS aggregate | `fastlane.blsAggregate(id)` |
| Pending payloads | `fastlane.pendingCount()` |
| Next nonce | `fastlane.nonces(acct)` |
| A bond | `fastlane.authorityBonds(acct)` |
| All payloads | `fastlane.statuses.entries()` |

Node health over plain HTTP:

```bash
curl -s -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"system_health"}' http://127.0.0.1:9944
```

### 18.8 Reading OCW activity in the logs

Node logs are in `/root/my-blockchain/data/<name>.log`. The OCW itself logs nothing (the code has no `log::` calls), so you observe it through its effects: transactions in the pool and `Attestations` growing. **[RECOMMENDED CHANGE]** add `log::info!(target: "fastlane-ocw", ...)` at: key found, payload considered, tx submitted, submit error.

---

## 19. How to build this pallet from scratch

A condensed recipe that mirrors what this repo does.

### Step 1: Scaffold

```bash
cargo new --lib pallets/fastlane
```

`Cargo.toml` essentials: `frame-support`, `frame-system`, `sp-io`, `sp-runtime`, `sp-core`, `sp-std`, `parity-scale-codec`, `scale-info`, and for BLS `w3f-bls` (no_std). Feature `std` forwards to each.

### Step 2: Config trait

Include: `RuntimeEvent`, `Currency: ReservableCurrency`, `MaxAuthorities`, `MaxPendingPayloads`, `MinAuthorityBond`, `AuthorityId: AppCrypto`, `FinalizeDelay`, hooks (`OnFinalised`, `OffchainChecks`, `KycProvider`, `DomainRules`), `WeightInfo`.
The trait must also bound `CreateSignedTransaction<Call<Self>> + CreateInherent<Call<Self>>` so the OCW can build transactions.

### Step 3: Storage, events, errors

Start with the six spec items (section 4 of the spec), then add what you need: `Nonces`, `PendingCount`, `ExpiryAt`, `FinalizeAt`.

**Design rule:** never iterate unbounded maps in hooks. Use block-indexed schedules (`ExpiryAt[block]`) so each block touches a bounded bucket. This is why `ExpiryAt` and `FinalizeAt` exist.

### Step 4: Extrinsics

Write `submit`, `attest`/`attest_unsigned`, `finalize`, `set_authorities`, `set_threshold`, `post_bond`, `slash`. Share the logic in a `do_attest` helper so signed and unsigned paths behave identically.

### Step 5: Key type and crypto

```rust
pub mod crypto {
    pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"fast");
    app_crypto!(sr25519, KEY_TYPE);
    pub struct FastlaneAuthId;
    impl AppCrypto<MultiSigner, MultiSignature> for FastlaneAuthId {
        type RuntimeAppPublic = Public;
        type GenericSignature = sp_core::sr25519::Signature;
        type GenericPublic   = sp_core::sr25519::Public;
    }
}
```

### Step 6: `ValidateUnsigned`

For every unsigned call, **verify authenticity in the pool**: decode the signer, check membership, check freshness, **verify the signature**, and provide a dedupe tag.

### Step 7: Offchain worker

```rust
fn offchain_worker(n: BlockNumberFor<T>) {
    // 1. dedupe per block (local storage)
    // 2. find local keys: sp_io::crypto::sr25519_public_keys(KEY_TYPE)
    // 3. for each pending payload: run checks, sign msg_hash,
    //    build Call::attest_unsigned, T::create_inherent(call.into()),
    //    SubmitTransaction::<T, Call<T>>::submit_transaction(xt)
}
```

Keep it short, bounded and idempotent. Never panic.

### Step 8: Hooks

`on_initialize` for expiry and auto-finalise using block-indexed buckets and per-block caps.

### Step 9: Wire into the runtime

In `runtime/src/lib.rs` implement the offchain traits (`SigningTypes`, `CreateTransactionBase`, `CreateInherent`, `CreateSignedTransaction`) for `Runtime`, add the pallet to the runtime, give it a `Config` impl, and set genesis authorities and threshold.

### Step 10: Node wiring

In `node/src/service.rs` start `sc_offchain::OffchainWorkers` with the keystore and a transaction pool factory when `config.offchain_worker.enabled`.

### Step 11: Operations

Keys inserted before start; `--offchain-worker always`; bond and register authorities.

### Step 12: Tests and benchmarks

Mock runtime, unit tests per extrinsic, OCW test with `TestOffchainExt` / `TestTransactionPoolExt`, benchmarks to generate `weights.rs`.

---

## 20. Known problems and recommended fixes

Ordered by importance. Nothing here has been changed in the code.

### Issue 1: PreConsensus is 6–12 s, not under 1 s

**What:** spec section 13 expects under 1 s; this design needs block inclusion first (section 8.9).

**Options:**

| Option | Idea | Effort |
|---|---|---|
| A. Update the spec | "PreConsensed within 1–2 blocks" | trivial |
| B. Off-chain gossip aggregator | authorities sign over a p2p/RPC channel; a collector gathers `Threshold` signatures in ~100s of ms and submits **one** extrinsic `submit_preconsensus(payload_id, sigs)` (or one BLS aggregate). UI trusts the aggregator's off-chain proof for instant UX, chain confirms next block | large |
| C. Shorter blocks | 1–2 s slot time reduces but does not remove the problem | small, but affects the whole chain |

### Issue 2: Off-chain checks are mostly disabled

`KycProvider = ()`, `DomainRules = ()`, `SubmitCooldown = 0`, `rate_limit_ok = true`. Implement real providers in the runtime and enable a non-zero cooldown. For genuinely off-chain checks (HTTP KYC), restructure `checks_hash` so the chain verifies a quorum of identical hashes rather than recomputing it.

### Issue 3: attest_unsigned signature is not verified in the pool

[lib.rs:981](../pallets/fastlane/src/lib.rs#L981) only checks `!signature.is_empty()`.

**Impact:** a forged tx naming an authority passes pool validation, occupies the dedupe tag `(payload_id, authority)`, and the real attestation is dropped as a duplicate until the forged one is included and fails in `do_attest`. Repeating it each block can delay (not forge) pre-consensus. Block authors also waste block space on failing extrinsics.

**[RECOMMENDED CHANGE]** inside the `attest_unsigned` branch of `validate_unsigned`, after the `checks_hash` check:

```rust
let mut msg = payload_id.to_vec();
msg.extend_from_slice(&payload.expiry.encode());
msg.extend_from_slice(checks_hash);
let msg_hash = sp_io::hashing::blake2_256(&msg);
Self::verify_authority_sig(&authority, &msg_hash, signature)
    .map_err(|_| InvalidTransaction::BadProof)?;
```

Add a unit test with a wrong signature expecting `BadProof`.

### Issue 4: OCW scans every status each block

`Statuses::<T>::iter()` is O(all payloads ever), because finalised and expired entries are never removed. Currently 26 entries, so harmless; it will not scale.

**Fix:** keep a bounded `PendingPayloads: BoundedVec<PayloadId, MaxPendingPayloads>` (push on `submit`, remove on PreConsensed/Expired), and iterate that. Alternatively prune old `Statuses`/`Payloads` after N blocks.

### Issue 5: Stale comments

The comment block above `offchain_worker` describes `Signer::all_accounts()` and "FIX" history. The code uses `sp_io::crypto::sr25519_sign`. Update the comments.

### Issue 6: DoubleSigning proof is meaningless

`validate_slash_proof` treats "signed two different payloads" as double-signing. Every honest authority signs many payloads. With sudo this lets the key holder slash anyone "with proof". Real double-signing must mean **conflicting statements for the same slot**, such as two different `checks_hash` for the same `payload_id`, or two payloads with the same `(creator, nonce)` but different `payload_hash`. Also, the pallet currently makes the second case impossible, since one `(creator, nonce)` yields one id.

**Fix:** either remove `DoubleSigning` until a conflicting-statement model exists, or redefine it as two valid signatures by the same authority over the same `payload_id` with different `checks_hash`.

### Issue 7: Failed downstream hook strands the payload

In `on_initialize`, if `OnFinalised::on_finalised` returns `Err`, the payload is dropped from `FinalizeAt` and stays `PreConsensed`, with no event.

**Fix:** emit a `FinaliseFailed{payload_id}` event and reschedule with a retry counter and cap, or leave a documented manual path.

### Issue 8: Gate does not bind payload to call data

Section 13.4. Production pallets must bind `creator`, `payload_hash` and one-time use.

### Issue 9: No bond withdrawal

No `unbond`. Add a delayed unbond that fails if the account is an active authority or has open commitments.

### Issue 10: Genesis does not validate threshold or bonds

`GenesisConfig::build` should assert `threshold > len/2 && threshold <= len` and that every bond reserve succeeded (it silently ignores failures).

### Issue 11: `set_authorities` can strand the chain

No check that the new length is at least `Threshold`. Add `ensure!(len >= Threshold)` or auto-lower the threshold with an event.

### Issue 12: Dev keys are used for real authorities

The live authorities are the well-known dev accounts (`//Alice`, `//Bob`, `//Charlie`, `//Dave`), and the same secret is used for the `fast`, `aura` and `gran` keys. Anyone who knows those public derivation paths can sign as these authorities. See [section 22](#22-security-and-operations).

### Issue 13: Attestation message has no chain binding

Add `genesis_hash` (and `spec_name`) into the signed message to prevent cross-chain replay.

### Issue 14: Misleading error names

`InvalidSignature` is returned when `compute_checks_hash` yields `None` (a failed business check) or `checks_hash` mismatches. Add `ChecksFailed` and `ChecksMismatch`.

### Issue 15: Unbounded expiry

Add `MaxExpiryWindow` (for example 30 blocks as the spec recommends): `ensure!(expiry <= now + MaxExpiryWindow)`. This also shrinks the window in which a pending slot can be held hostage.

### Summary table

| # | Issue | Severity | Effort |
|---|---|---|---|
| 3 | Pool doesn't verify signature | High | Small |
| 12 | Public dev keys on live authorities | High (operations) | Medium |
| 6 | Meaningless double-sign proof | High (with sudo) | Small |
| 8 | Gate not bound to call data | High for real use | Medium |
| 2 | Checks disabled | Medium–High | Medium |
| 7 | Hook failure strands payload | Medium | Small |
| 11 | Authority set can drop below threshold | Medium | Small |
| 10 | Genesis not validated | Medium | Small |
| 15 | Unbounded expiry | Medium | Small |
| 9 | No unbond | Medium | Medium |
| 4 | Full scan in OCW | Medium (future) | Small |
| 13 | No chain binding | Low–Medium | Small |
| 14 | Error naming | Low | Trivial |
| 5 | Stale comments | Low | Trivial |
| 1 | Latency vs. spec | Decision | Large |

---

## 21. Testing

### 21.1 Existing tests

| File | Covers |
|---|---|
| `pallets/fastlane/src/tests.rs` | submit, attest, thresholds, finalise, expiry, governance, slash |
| `pallets/fastlane/src/bls_aggregation_tests.rs` | BLS register, attest, aggregate, verify |
| `pallets/template/src/tests.rs` | gated call behaviour |
| `scripts/e2e-phase2.js` | live-chain end-to-end |
| `scripts/bls-live-demo.js`, `bls-ocw-demo.js` | BLS flows on a live chain |

Run:

```bash
cargo test -p pallet-fastlane
cargo test -p pallet-fastlane --features runtime-benchmarks   # benchmark tests
```

### 21.2 Tests worth adding (matching the issues above)

1. `validate_unsigned` rejects a bad signature (issue 3).
2. `set_authorities` below threshold is rejected (issue 11).
3. Genesis with `threshold <= len/2` fails to build (issue 10).
4. Failed `on_finalised` emits an event and retries (issue 7).
5. `submit` with expiry beyond the max window is rejected (issue 15).
6. Gate: wrong creator or wrong hash is rejected (issue 8).
7. OCW unit test using `TestOffchainExt`, `TestTransactionPoolExt` and a `KeystoreExt` containing a `fast` key. Assert one `attest_unsigned` per pending payload and none for a payload already attested.

### 21.3 Manual smoke test on a live chain

1. `fastlane.pendingCount()` → note.
2. Submit via script (18.2).
3. Within ~12 s: `PreConsensed`; within ~24 s: `Finalised`.
4. `fastlane.aggregatedAttestations(id)` → `count == 3`.
5. Submit with `expiry = now + 2` while stopping 2 authority nodes (`pm2 stop fastlane-charlie fastlane-dave`) → status becomes `Expired`; restart nodes.

---

## 22. Security and operations

### 22.1 Threat model in one table

| Threat | Mitigation present | Gap |
|---|---|---|
| Forged attestation | `do_attest` verifies sr25519 | pool doesn't (issue 3) |
| Duplicate / replay | nonce, `payload_id`, per-authority dedupe | no chain binding (issue 13) |
| Pending-queue flooding | `MaxPendingPayloads`, `SubmitCooldown` | cooldown = 0; no fee beyond tx fee; no expiry cap |
| Malicious authority | bond + slash | proofs weak (issue 6); sudo only |
| Authority collusion | threshold > 50% | 3 of 4 colluders can approve anything |
| Authority key theft | none | dev keys public (issue 12) |
| Treating PreConsensed as final | docs only | UI must show it as "approved, not final" |
| Reorg | GRANDPA | `PreConsensed` can revert with its block |

### 22.2 Exposure of the live nodes

The run scripts for Alice, Bob, Charlie, Dave and Eve use `--unsafe-rpc-external` with `--rpc-methods safe`. Safe methods can't insert keys, but they do expose the full read API and can **submit extrinsics** from any client. Your server notes record a root compromise on 2026-10-07 and a pending reinstall plus secret rotation. For this stack:

1. Confirm `ufw` allows only the ports you intend (9944–9948 and 30333–30337 should be reachable only from trusted hosts, or from nginx on localhost).
2. Prefer `--rpc-external` behind an nginx TLS proxy, or bind to `127.0.0.1`.
3. Treat the `data/*` keystores as **compromised** until rotated (keys are plaintext on disk; the scripts' own README says the volume must be encrypted).

### 22.3 Key hygiene (production checklist)

1. Generate a fresh key per authority: `solochain-template-node key generate --scheme sr25519`.
2. Use a **separate** `fast` key from the account that holds funds; only the `fast` public key goes in `Authorities` (this needs a small code change, since today the account **is** the key).
3. Store SURIs in a secret manager, not in shell history.
4. Replace the fixed `--node-key` in `run-alice.sh`.
5. Replace sudo with a council/referendum origin before any real value is at stake.
6. Back up `offchain` storage? The BLS seed is node-local; if lost, the node simply registers a **new** BLS key at next start (the OCW detects the mismatch and re-registers). Past aggregates remain verifiable with the old keys only if you archive them.

### 22.4 Monitoring

Alerts worth having:

- `PendingCount` stuck above 0 for more than `expiry` blocks → authorities not attesting.
- A `Submitted` payload older than 3 blocks → OCW problem.
- Count of `Expired` events per hour → committee health.
- Nodes' best-block spread → network partition.
- `Authorities.len() < Threshold` → chain cannot pre-consense.

---

## 23. Troubleshooting

| Symptom | Likely cause | Check / fix |
|---|---|---|
| Payload stays `Submitted` and then `Expired` | OCWs not attesting | next rows |
| No attestations at all | node has no `fast` key | `ls <base-path>/chains/*/keystore/`; run `setup-keys.sh`; restart. The old `--dev` pm2 node had a temporary keystore and this exact symptom |
| Node has key but nothing happens | `--offchain-worker` not `always`/enabled | check the process arguments (`ps aux \| grep solochain`) |
| Key present but account isn't an authority | not in `Authorities` | `fastlane.authorities()`; add via `set_authorities` |
| Authority has a bond but isn't attesting | bonded ≠ authority | `post_bond` only reserves; root must add |
| Only 2 of 3 required signatures arrive | an authority node is down / out of sync | `system_health` on each port; `pm2 status` |
| `submit` fails `InvalidNonce` | nonce mismatch | read `fastlane.nonces(acct)` just before sending |
| `submit` fails `TooManyPendingPayloads` | 100 pending, or expiry bucket full | wait for expiries; use a different expiry block |
| `submit` fails `PayloadExpired` | `expiry <= current block` | set `expiry = best + 20` |
| `submit` fails `SubmitTooFrequent` | cooldown configured | wait or set cooldown to 0 |
| `attest*` fails `InvalidSignature` after a checks change | `checks_hash` mismatch or `compute_checks_hash` returned `None` | creator's balance is zero, domain disallowed, KYC failed |
| `AlreadyPreConsensed` errors in blocks | late attestations (normal) | ignore |
| Stuck in `PreConsensed` | `FinalizeAt` bucket was full, or `on_finalised` errors | call `fastlane.finalize(id)` manually (issue 7) |
| `set_threshold` fails `ThresholdTooLow` | `value <= len/2` | pick `len/2 + 1` or more |
| `set_threshold` fails `ThresholdTooHigh` | `value > len` | lower it |
| `set_authorities` fails `BondTooLow` | member without bond | `post_bond` ≥ 1000 first |
| After a slash, no payload completes | evicted authority; `len < Threshold` | `set_authorities` + `set_threshold` |
| BLS aggregate missing | BLS keys not registered yet | wait 2–3 blocks after start; `authorityBlsKey(acct)` |
| Build fails in RocksDB / `cstdint` | GCC 15 | `export CXXFLAGS="-include cstdint"` and rebuild |
| Node log shows tx pool rejection `BadProof` | authority not in set / checks mismatch / BLS signature invalid | compare `Authorities` and the node's key |
| Runtime upgrade seems ignored | `spec_version` not bumped | bump and re-submit |
| Node won't start: DB errors | big RocksDB (dave ~13 GB) slow to open | wait; pm2 `min_uptime` is 90 s for this reason |

Quick diagnostic commands:

```bash
pm2 status
pm2 logs fastlane-alice --lines 100
ls /root/my-blockchain/data/alice/chains/*/keystore | head
curl -s -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"system_health"}' http://127.0.0.1:9944
```

---

## 24. Cheat sheet

### States

```
Submitted ──≥Threshold attestations──► PreConsensed ──+FinalizeDelay blocks / finalize()──► Finalised
    └──── now > expiry ───────────────► Expired
```

### Formulas

```
payload_id = blake2_256( SCALE(creator, nonce:u64, domain:u32, payload_hash:[u8;32]) )
msg_hash   = blake2_256( payload_id ++ SCALE(expiry) ++ checks_hash )
signature  = sr25519_sign(fast_key, msg_hash)                       // 64 bytes
inclusion  = blake2_256( payload_id ++ "INCLUDE" ++ SCALE(deadline) )
bls_msg    = Message("fastlane-attest", payload_id)
```

### Limits and values (live)

| Name | Value |
|---|---|
| Block time | 6 s |
| Authorities / threshold | 4 / 3 |
| Max authorities | 50 |
| Max pending payloads | 100 |
| Min bond | 1000 |
| Finalize delay | 2 blocks |
| Auto-finalise / expiry per block | 50 / 50 |
| Unsigned tx longevity | 5 blocks, priority 100 |

### Who can call what

| Call | Who |
|---|---|
| `submit`, `finalize`, `post_bond` | any signed account |
| `attest`, `commit_inclusion`, `register_bls_key`, `attest_bls` | the authority itself (signed) |
| `attest_unsigned`, `attest_bls_unsigned`, `register_bls_key_unsigned` | nobody manually; OCW only (unsigned, pool-validated) |
| `set_authorities`, `set_threshold`, `slash` | root (sudo) |

### Where to look

| I want to… | Open |
|---|---|
| Understand the OCW | [lib.rs:798](../pallets/fastlane/src/lib.rs#L798) |
| Change what is checked | [lib.rs:102](../pallets/fastlane/src/lib.rs#L102) |
| Change thresholds/limits | [configs/mod.rs:176](../runtime/src/configs/mod.rs#L176) |
| Change initial committee | [genesis_config_presets.rs](../runtime/src/genesis_config_presets.rs) |
| Start the OCW runner | [service.rs:187](../node/src/service.rs#L187) |
| Add keys | [scripts/setup-keys.sh](../scripts/setup-keys.sh) |
| Run nodes | [scripts/run-alice.sh](../scripts/run-alice.sh), [ecosystem.config.js](../ecosystem.config.js) |
| Test end to end | [scripts/e2e-phase2.js](../scripts/e2e-phase2.js) |

### One-paragraph answer to "is the offchain worker on-chain?"

The OCW **code** is part of the on-chain runtime, but it **runs off-chain**, privately on each node, after every imported block. It can read chain state and sign with that node's local keys, but it can only affect the chain by submitting a transaction. That transaction is validated by the pallet's `validate_unsigned` and executed on-chain by `do_attest`. Only that on-chain execution counts toward the threshold.
