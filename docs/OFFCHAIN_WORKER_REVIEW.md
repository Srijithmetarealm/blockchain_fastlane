# FastLane Offchain Worker: How It Works and Where It Differs From the Spec

Scope: `pallets/fastlane/src/lib.rs`, `runtime/src/configs/mod.rs`, `node/src/service.rs`, `ecosystem.config.js`.
Reviewed against: the Pre-Consensus Attestation Pallet specification (§6.2, §7, §9, §13).
Status: review only. **No code was changed.**

---

## 1. Is the offchain worker on-chain or off-chain?

**Both.** The two parts are separate:

| Part | Where it lives | Runs where |
|---|---|---|
| `offchain_worker()` code | Inside the runtime Wasm ([lib.rs:798](../pallets/fastlane/src/lib.rs#L798)) | **Off-chain**, privately on each node, after each imported block |
| `attest_unsigned` and friends | Pallet extrinsics | **On-chain**, executed by consensus in a block |
| Signing keys (sr25519 `fast`, BLS seed) | Node keystore / offchain-local storage | **Off-chain only**, never on-chain |

The worker **cannot write chain state**. It can only read state and submit a transaction. That transaction is then validated by the pool (`validate_unsigned`) and applied on-chain by `do_attest`.

## 2. Flow

```
block N imported
  └─ offchain_worker(N)                       [off-chain, per node]
       1. skip if block already processed (fastlane::last_ocw_block)
       2. stop if node has no "fast" sr25519 key
       3. ensure own BLS public key is registered on-chain
       4. for each Submitted payload, for each local key that is an authority:
            msg = blake2_256(payload_id ‖ expiry ‖ checks_hash)
            sig = sr25519_sign(msg)
            submit unsigned tx attest_unsigned(...)      (+ attest_bls_unsigned)
  └─ tx pool: validate_unsigned                [on-chain rules, pre-inclusion]
  └─ block N+1: attest_unsigned -> do_attest   [on-chain]
       verify sr25519 signature, store attestation
       count >= Threshold  ->  PreConsensed, schedule FinalizeAt(now + 2)
  └─ on_initialize                              [on-chain]
       Submitted past expiry -> Expired
       PreConsensed due      -> Finalised
```

Live setup: 5 nodes under pm2 (alice, bob, charlie, dave, eve), all with `--offchain-worker always`. Alice and Bob are block-producing validators. The others still attest if their keystore holds an authority `fast` key.

## 2.1 The offchain worker in detail

### What an offchain worker is
An offchain worker (OCW) is a function in the runtime (`fn offchain_worker(block_number)`) that each node runs **after it imports a new best block**. It runs outside consensus, so each node can do different things, use local secrets, and take a little time. Its result is never checked directly. The only way it affects the chain is by submitting a transaction.

| Question | Answer |
|---|---|
| Is the code on-chain? | Yes. It is in the runtime Wasm and changes with a runtime upgrade (no node rebuild needed) |
| Does it run on-chain? | No. It runs privately on each node |
| Can it write chain state? | No. It can read state and submit transactions |
| Does it pay fees? | Not for unsigned transactions. The pallet's `validate_unsigned` decides if they are accepted |
| Is it deterministic? | No. Different nodes may submit different things, or nothing |

### How the node starts it
[node/src/service.rs:187-204](../node/src/service.rs#L187) starts `sc_offchain::OffchainWorkers` with the node's keystore, offchain database and transaction pool, but only when `--offchain-worker` is enabled. The run scripts pass `--offchain-worker always`, so even non-validator nodes (Charlie, Dave, Eve) run it. The default `when-authority` would skip them.

### What it does on each block ([lib.rs:798](../pallets/fastlane/src/lib.rs#L798))
1. **Per-block dedupe.** It reads the local key `fastlane::last_ocw_block`. If it already ran for this block number or a later one, it stops. This prevents double submission after a restart.
2. **Find local keys.** It lists sr25519 keys of type `fast` in the node keystore. No key means it silently does nothing. This is the most common reason attestations never appear.
3. **Register the BLS key.** For each local key that is an authority, it makes sure the node's own BLS public key is registered on-chain, sending `register_bls_key_unsigned` if it is not.
4. **Scan payloads.** It walks `Statuses` and picks every `Submitted` payload.
5. **Run checks.** `compute_checks_hash` returns `None` if any check fails, and the worker skips that payload. Otherwise it gets a `checks_hash`.
6. **Sign.** For each local authority key that has not attested yet, it signs `blake2_256(payload_id ++ expiry ++ checks_hash)` with `sr25519_sign`.
7. **Submit.** It builds `attest_unsigned` (authority, 64-byte signature, `checks_hash`) and pushes it into the local transaction pool, which gossips it to peers. It also submits `attest_bls_unsigned` once its BLS key is registered.

On-chain, `validate_unsigned` admits the transaction to the pool, and `do_attest` verifies the signature and counts it when a block author includes it.

### Keys it uses
| Key | Where it lives | Created by | Used for |
|---|---|---|---|
| sr25519, key type `fast` | node keystore | `scripts/setup-keys.sh` | signing attestations; its public key is the authority's account |
| BLS seed (random 32 bytes) | offchain-local storage, key `fastlane::bls-seed::<account>` | generated by the worker on first use | signing BLS attestations |

Neither secret ever goes on-chain. Only the BLS **public** key is registered.

### What it does not do
- It does not read events. It scans storage.
- It does not talk to other authorities directly. There is no off-chain signature gossip.
- It does not retry explicitly. The next block's run re-checks `Attestations` and submits again if needed.
- It does not post inclusion commitments.
- It writes no logs, so you only see its effects (transactions in the pool, `Attestations` growing).
- It does no HTTP calls, so KYC or risk checks against outside services are not possible today.

### When it fails silently
| Symptom | Cause |
|---|---|
| No attestations at all | node has no `fast` key (the old `--dev` pm2 node had exactly this problem) |
| Key exists, still nothing | account is not in `Authorities` (Eve is in this state) |
| Nothing after a re-org | the per-block dedupe key is higher than the new block number until the chain passes it |
| Payload skipped | a check in `compute_checks_hash` failed (zero balance, domain, KYC) |

### Live state (checked 2026-10-09)
The chain has 4 authorities (Alice, Bob, Charlie, Dave), threshold 3. Eve runs a worker but has no matching authority, so it submits nothing. All 26 stored payloads are `Finalised`.

## 3. Spec vs. implementation

| Spec item | Implemented? | Notes |
|---|---|---|
| §6.2 `attest` as an unsigned tx with a signed payload | Yes | `attest_unsigned`; a signed `attest` also exists for testing |
| §6.2 only valid if signer is an authority, status is Submitted, and not expired | Yes | Checked in both `validate_unsigned` and `do_attest` |
| §6.2 threshold reached -> PreConsensed | Yes | [lib.rs:1603](../pallets/fastlane/src/lib.rs#L1603) |
| §7 authorities sign `payload_id + expiry + checks_hash` | Yes | Same bytes are rebuilt and verified on-chain |
| §7 validation: signature, nonce, balance, KYC, rate limit, domain rules | **Partial** | See issue 2 |
| §9 expiry in `on_initialize`; PreConsensed cannot expire | Yes | Uses an `ExpiryAt` schedule and a status guard |
| §13 PreConsensus < 1 second | **No** | See issue 1 |

## 4. Issues found

### Issue 1: PreConsensus is not sub-second (spec §13)
The worker is triggered by block import, and its attestation counts only once included in a later block. Realistic time to `PreConsensed` is **1–2 blocks (~6–12 s at 6 s blocks)**. With `FinalizeDelay = 2`, finalisation follows about 2 blocks later.
Reaching < 1 s requires gossiping signatures off-chain between authorities (a separate p2p/RPC layer). The `offchain_worker` hook alone cannot do this.

### Issue 2: Off-chain checks are effectively on-chain checks, and some are disabled
The chain recomputes `checks_hash` and requires it to match, so the worker can only check what the chain can also compute: domain, nonce, balance, KYC and domain rules.
In the runtime configuration:
- `KycProvider = ()` -> everyone passes KYC
- `DomainRules = ()` -> everything allowed
- `SubmitCooldown = 0` -> rate limiting off
- `OffchainChecks = ()` -> default checks only

So spec §7's KYC, rate-limit and domain-specific rules are **not enforced** today.

### Issue 3: Pool-level validation does not verify the signature
`validate_unsigned` for `attest_unsigned` only checks the signature is non-empty ([lib.rs:981](../pallets/fastlane/src/lib.rs#L981)). Real verification happens later in `do_attest`. The pool dedupes on `(payload_id, authority)`, so a forged tx naming an authority can occupy that slot and delay the genuine attestation.
(The BLS variants already verify the signature in `validate_unsigned`, so the sr25519 path is the inconsistent one.)

### Issue 4: Worker scans all payload statuses every block
`Statuses::<T>::iter()` walks the entire map. Cost grows with history unless finalised entries are pruned.

### Issue 5: Stale comments
The comment above `offchain_worker` describes `Signer::all_accounts()`, but the code calls `sp_io::crypto::sr25519_sign` directly. `create_inherent` is the SDK's name for building a bare (unsigned) extrinsic; it is not a real inherent.

## 5. Recommended changes

| # | Change | Effort | Priority |
|---|---|---|---|
| 3 | Call `verify_authority_sig` inside `validate_unsigned` for `attest_unsigned` | Small | High |
| 2 | Provide real `KycProvider`, `DomainRules`, `SubmitCooldown` for production | Medium | High before mainnet |
| 4 | Index Submitted payloads (e.g. a bounded `PendingPayloads` list) instead of iterating all statuses | Small–Medium | Medium |
| 1 | Add off-chain signature gossip if sub-second is a hard requirement; otherwise update the spec to say "1–2 blocks" | Large | Decision needed |
| 5 | Fix outdated comments | Trivial | Low |

## 6. Open question for the team lead

Is the < 1 second target (§13) a hard requirement? If yes, issue 1 needs a new design. If it is a goal, the spec wording should change to match the current 1–2 block behaviour.
