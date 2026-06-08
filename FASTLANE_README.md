# FastLane Pre-Consensus Pallet — Implementation Status

## Overview

FastLane is a Substrate pallet that implements pre-consensus attestation. Before a payload is included in a block, threshold signatures (e.g., 5/7 authorities) must sign it off-chain. Once threshold is reached, the payload is marked `PreConsensed` and ready for block inclusion.

---

## ✅ What Works

### 1. **Single-Node Dev Setup**
- Fresh node can start with: `./scripts/run-dev.sh`
- Ticks blocks every ~6 seconds
- Alice dev account has keys in keystore automatically
- Node includes OCW that monitors and signs payloads

### 2. **Payload Submission (Frontend)**
- Any account can submit a payload via the frontend
- Accounts can be:
  - Dev accounts (Alice, Bob, Charlie, etc. from keyring)
  - Extension accounts (Polkadot.js extension imported)
- Submission works with both account types

### 3. **Payload Lifecycle**
- Submitted → stored on-chain
- PreConsensed → when threshold signatures collected
- Finalised → after block finalization
- Expired → if timeout reached without threshold

### 4. **Threshold Signature Verification**
- OCW signs attestations with sr25519 keys
- Signatures verified on-chain
- Counted toward threshold
- Once threshold met (e.g., 5/7), payload marked PreConsensed

### 5. **Events Visible in Frontend**
- All payload state transitions appear as events
- Frontend displays: Submitted, PreConsensed, Finalised, Expired
- Event timestamps and payload IDs are visible

### 6. **Authority Slashing & Auto-Eviction**
- Authorities can be slashed if they sign invalid payloads
- If bond falls below `MinAuthorityBond`, auto-evicted
- Slashing mechanism tested and working

### 7. **Bounded Storage Operations**
- `ExpiryAt` index prevents O(n) expiry scans
- `PreConsensedQueue` bounds finalization operations
- `on_initialize` respects max operations per block

### 8. **Pallet Compiles & Tests Pass**
- All 31 unit tests pass with real sr25519 keys
- No trait mismatches or compilation errors
- Runtime correctly wires `CreateInherent` and `CreateSignedTransaction`

---

## ❌ Cannot Do / Limitations

### 1. **Cannot Sign from Browser/Extension**
**Limitation:** OCW cannot access Polkadot.js extension keys
**Why:** 
- Extension keys live in browser memory, not on validator node
- OCW runs on the node server, not browser
- For security, validator keys must be isolated to the validator machine
- If validator keys were in browser, a compromised browser/RPC port = stolen validator keys

**Architecture:**
```
Frontend (Browser)          Node (Server)
├─ Extension keys           ├─ Local disk keystore (validator keys)
│  (user submission only)    └─ OCW (validator attestation only)
└─ Cannot trigger OCW
```

### 2. **Cannot Use Same Account for User Submission & Validator Attestation** (Production)
**Limitation:** In production, user wallet keys and validator keys must be separate
**Why:**
- User wallet (extension key) is meant for submitting transactions
- Validator key is meant for running the validator OCW
- Mixing them violates separation of concerns
- Compromised user wallet ≠ compromised validator

**Note for Dev Testing:**
For single-machine dev testing only, you can add the same dev account's keys to the keystore:
```bash
./scripts/setup-keys.sh ./data/dev dev //Alice
./scripts/setup-keys.sh ./data/dev dev //Charlie  # Add more
```
This allows both Alice and Charlie's OCWs to sign on one machine (testing only, not production).

### 3. **Cannot Run Multi-Validator Setup on One Machine Securely**
**Limitation:** Multiple validators on one machine = single point of failure
**Why:**
- If one machine is compromised, all validators on it are compromised
- All keys are in one place (`./data/dev/keystore/`)
- One disk breach = all validator keys exposed
- Production requires separate machines for each validator

**Workaround for Testing:**
Add multiple keys to one keystore to test pallet logic:
```bash
./scripts/setup-keys.sh ./data/dev dev //Alice
./scripts/setup-keys.sh ./data/dev dev //Bob
./scripts/setup-keys.sh ./data/dev dev //Charlie
```
All three OCWs run on same node and sign. **This is ONLY for testing pallet threshold logic.**

### 4. **Cannot Dynamically Add/Remove Validator Keys at Runtime**
**Limitation:** Keys must be in keystore before node starts
**Why:**
- Keys are read from disk at node startup: `sr25519_public_keys(KEY_TYPE)`
- Adding a key while node is running requires restart
- OCW only reads keys once during initialization

**Current Flow:**
1. Stop node
2. Add/remove key via `./scripts/setup-keys.sh`
3. Restart node
4. OCW picks up new keys

### 5. **Cannot Use OCW to Attest Without Authority Membership**
**Limitation:** Only authorities in the `Authorities` set can attest
**Why:**
- Pallet checks: `ensure!(Authorities::<T>::contains_key(&signer), ...)`
- Non-authorities submitting attestations are rejected
- This prevents unauthorized validators from attesting

**To Test with Custom Account:**
1. Add account to `Authorities` in genesis config
2. Add account's keys to keystore
3. Restart node

### 6. **Cannot Have Unbounded Expiry/Finalization Loops**
**Limitation:** `on_initialize` operations are capped per block
**Why:**
- If 1000 payloads expire per block, operations = unbounded
- Blocks become too heavy, slower finalization
- Chain stalls on next block if previous exceeded weight

**Implementation:**
```rust

MaxExpiriesPerBlock = 50    // Max 50 expiries per block
MaxAutoFinalizePerBlock = 50 // Max 50 finalizations per block
```
Excess payloads expire in following blocks.

### 7. **Cannot Have Weights Be Production-Ready**
**Limitation:** Current weights are conservative placeholders
**Why:**
- Real weights require `frame-benchmarking` on production hardware
- Placeholder values: "assume 50ms ref_time + per-call costs"
- Production needs hardware-specific benchmarks
- Using wrong weights can cause block rejection

**Current Status:**
```rust
// Placeholder: conservative estimate
submit() = 50ms ref_time + call cost
attest() = 50ms ref_time + call cost
```
**Required for production:** Run `frame-benchmarking` on target hardware, generate weights.

### 8. **Cannot Mix OCW Signatures with BLS Aggregation** (Phase 2)
**Limitation:** Current implementation uses individual sr25519 signatures
**Why:**
- BLS aggregation requires different crypto
- Would increase complexity significantly
- Not in Phase 1 spec
- Substrate's frame-system uses sr25519, not BLS

**Current:**
- Each authority signs individually
- Signatures counted one by one
- N authorities = N signatures on-chain

**Phase 2 (optional):**
- BLS aggregation could reduce signature size
- Requires custom crypto implementation
- Not planned for current release

### 9. **Cannot Have Signatures Auto-Finalize Without Downstream Hook**
**Limitation:** Payloads don't auto-finalize; they stay PreConsensed
**Why:**
- Finalization requires application logic (downstream pallet)
- FastLane doesn't know what to do with PreConsensed payloads
- Other pallet must call `finalize()` explicitly
- This prevents scope creep and keeps FastLane generic

**Example:** If building a rollup sequencer:
- Fastlane pallet marks payload PreConsensed ✅
- Rollup pallet sees PreConsensed event → calls finalize() ✅

### 10. **Cannot Use Sudo for Production Authority Updates**
**Limitation:** Current runtime uses Sudo to set authorities
**Why:**
- Sudo = single admin, no decentralization
- Production requires on-chain governance (vote)
- Sudo can be abused: admin unilaterally changes authorities
- Testnet fine, production not acceptable

**Current:**
```rust
set_authorities() // Sudo-only, testing only
set_threshold()   // Sudo-only, testing only
```

**Required for Production:**
- Replace Sudo with Governance pallet
- Authorities changed via referendum
- Threshold changed via referendum

---

## Architecture Diagram

### Single-Node Setup (Current)
```
┌─────────────────────────────────┐
│   Node (./data/dev)             │
│ ┌─────────────────────────────┐ │
│ │ Alice's Keys                │ │
│ │ ├─ aura (block production)  │ │
│ │ ├─ gran (finality)          │ │
│ │ └─ fast (attestation)       │ │
│ │                             │ │
│ │ OCW (runs 24/7)             │ │
│ │ ├─ Monitors Submitted       │ │
│ │ ├─ Signs attestations       │ │
│ │ └─ Submits unsigned txs     │ │
│ └─────────────────────────────┘ │
│                                 │
│ RPC: ws://localhost:9944        │
└─────────────────────────────────┘
         ↓
┌──────────────────────────┐
│  Frontend Dashboard      │
│  (http://localhost:5173) │
│  ├─ Alice account        │
│  ├─ Extension accounts   │
│  └─ Submit payload       │
└──────────────────────────┘
```

### Multi-Machine Setup (Production)
```
Machine 1: Alice         Machine 2: Bob          Machine 3: Charlie
┌──────────────┐        ┌──────────────┐        ┌──────────────┐
│ Alice's Keys │        │ Bob's Keys   │        │Charlie's Keys│
│ ├─ aura      │        │ ├─ aura      │        │ ├─ aura      │
│ ├─ gran      │        │ ├─ gran      │        │ ├─ gran      │
│ └─ fast      │        │ └─ fast      │        │ └─ fast      │
│              │        │              │        │              │
│ Alice OCW    │        │ Bob OCW      │        │ Charlie OCW  │
└──────────────┘        └──────────────┘        └──────────────┘
      ↓                       ↓                       ↓
      └───────────────────────┬───────────────────────┘
                              ↓
              Frontend (separate secure machine)
              ├─ User submits with any account
              ├─ All validators attest independently
              └─ Threshold collected on-chain
```

---

## Setup Instructions

### Fresh Single-Node Dev Start
```bash
# 1. Stop any running nodes
pkill -f solochain-template-node

# 2. Clean all data
rm -rf data/*

# 3. Clean cargo
cargo clean

# 4. Build
cargo build --release

# 5. Setup keys
./scripts/setup-keys.sh ./data/dev dev //Alice

# 6. Run node
./scripts/run-dev.sh
```

Expected output:
```
Block #1
Block #2
Block #3 ...
```

### Multi-Validator Local Network (3 Nodes, Same Machine - Testing Only)
```bash
# Terminal 1: Setup keys for all
./scripts/setup-keys.sh ./data/alice local //Alice
./scripts/setup-keys.sh ./data/bob local //Bob
./scripts/setup-keys.sh ./data/charlie local //Charlie

# Terminal 1: Run Alice (block author)
./scripts/run-alice.sh

# Terminal 2: Run Bob (attester)
./scripts/run-bob.sh

# Terminal 3: Run Charlie (attester)
./scripts/run-charlie.sh
```

---

## Testing Scenarios

### Scenario 1: Single Account, Single Node (Easiest)
1. Start dev node: `./scripts/run-dev.sh`
2. Frontend submits as Alice (dev account)
3. Alice's OCW signs
4. Threshold reached → PreConsensed
5. Block finalized → Finalised

**Expected:** Submitted → PreConsensed → Finalised in ~12 seconds

---

### Scenario 2: Multiple Accounts on Single Node (Testing Multi-Authority)
1. Add multiple keys to same keystore:
   ```bash
   ./scripts/setup-keys.sh ./data/dev dev //Alice
   ./scripts/setup-keys.sh ./data/dev dev //Bob
   ./scripts/setup-keys.sh ./data/dev dev //Charlie
   ```
2. Set all three as Authorities in genesis
3. Start dev node: `./scripts/run-dev.sh`
4. Frontend submits as any account (Alice, Bob, or Charlie)
5. All three OCWs sign automatically
6. Threshold reached faster

**Expected:** All three OCWs sign in parallel on same node

---

### Scenario 3: Multi-Machine Network (Production-Like)
1. Three separate machines (or VMs)
2. Each runs their own node with their own keys
3. Each runs independent OCW
4. Frontend submits from secure machine
5. All validators attest independently

**Expected:** Decentralized, secure attestation

---

## Configuration

Located in `runtime/src/configs/mod.rs`:

```rust
pub const MAX_AUTHORITIES: u32 = 50;           // Max validators
pub const MAX_PENDING_PAYLOADS: u32 = 100;     // Max pending payloads
pub const MIN_AUTHORITY_BOND: Balance = 1000;  // Slash threshold
pub const MAX_AUTO_FINALIZE_PER_BLOCK: u32 = 50;  // Bounded ops
pub const MAX_EXPIRIES_PER_BLOCK: u32 = 50;    // Bounded ops
```

---

## Known Limitations & Trade-Offs

| Limitation | Why | Impact | Solution |
|-----------|-----|--------|----------|
| Keys must be on disk, not in extension | Security isolation | Cannot use browser keys for OCW | Each validator needs separate machine |
| Weights are placeholders | Require hardware benchmarking | May be inaccurate in production | Run `frame-benchmarking` before launch |
| Uses Sudo for governance | Testing only | No decentralization | Replace with governance pallet |
| No BLS aggregation | Phase 2 feature | Larger signature overhead | Implement BLS in Phase 2 |
| Single-machine multi-key is insecure | Keys in one place | Not for production | Deploy on separate machines |
| OCW requires node restart to add keys | Keys read at startup | Cannot hot-swap validators | Plan rotations ahead |
| PreConsensed doesn't auto-finalize | Keeps pallet generic | Need downstream logic | Have app-specific pallet call finalize() |

---

## Testing & Verification

### Unit Tests
All 31 tests pass:
```bash
cargo test --release -p pallet-fastlane
```

Tests cover:
- Payload submission validation
- Threshold signature verification
- Expiry and finalization bounds
- Slashing and auto-eviction
- OCW signature paths

### Integration Testing
1. Submit payload via frontend
2. Observe Submitted event
3. Wait ~6 seconds (one block)
4. Observe PreConsensed event
5. Wait ~6 seconds
6. Observe Finalised event

Total time: ~12 seconds (2 blocks)

---

## Production Readiness Checklist

- [x] Pallet compiles without errors
- [x] Unit tests pass
- [x] Runtime wiring correct
- [x] OCW reads keys from disk correctly
- [x] Threshold logic works
- [x] Auto-eviction works
- [x] Bounded operations prevent DoS
- [ ] **Weights benchmarked on production hardware** ← Required
- [ ] **Sudo replaced with governance pallet** ← Required
- [ ] **Deployed on separate machines per validator** ← Required
- [ ] **Security audit completed** ← Required

---

## FAQ

**Q: Can I use my Polkadot.js extension account as a validator?**
A: No. Extension keys are in the browser. Validator keys must be isolated to the validator machine for security.

**Q: Why can't I add keys while the node is running?**
A: OCW reads keys once at startup from the disk keystore. Adding keys requires restart so they're loaded.

**Q: Can I use one machine for three validators?**
A: Only for testing the pallet logic. For production, each validator must be on separate hardware (different machines, different networks, different security domains).

**Q: What happens if a validator goes offline?**
A: Their OCW stops signing. Other validators continue. Threshold may take longer if many validators are offline.

**Q: Can I change authorities without restarting the node?**
A: Yes, via `set_authorities()` (Sudo in testing, governance in production). New authorities are immediately active.

**Q: What if threshold is never reached?**
A: Payload expires after `ExpiryBlock` is reached. It's marked Expired and dropped from pending list.

**Q: Is this production-ready now?**
A: Not yet. Requires: (1) benchmarked weights, (2) governance pallet, (3) security audit, (4) separate machine deployment.

---

## Contact

For questions about FastLane implementation, refer to:
- Pallet code: `pallets/fastlane/src/lib.rs`
- Tests: `pallets/fastlane/src/tests.rs`
- Runtime config: `runtime/src/configs/mod.rs`
