// ============================================================
// scripts/fastlane-attestor.js
// Standalone replacement for the pallet's offchain worker. One instance per
// authority: watches for `Submitted` payloads and submits that authority's
// sr25519 `attest` and BLS `attest_bls` as SIGNED extrinsics.
//
//   AUTHORITY_URI=//Alice node scripts/fastlane-attestor.js ws://127.0.0.1:9944
//
// Env:
//   AUTHORITY_URI  sr25519 secret URI/seed of this authority      (required)
//   BLS_SEED_FILE  where the random 32-byte BLS seed is kept      (default ./.bls-seed-<addr>)
//   BLS_GEN        path to the built `bls_gen` example binary     (default target/release/examples/bls_gen)
//   DRY_RUN=1      log what would be submitted, send nothing
//
// IMPORTANT: run the node with `--offchain-worker never` first. Otherwise the
// node's OCW re-registers its own random BLS key and overwrites ours.
//
// Limitation: `checks_hash` is rebuilt here assuming the runtime's KYC and
// domain rules are allow-all (true on this testnet, see runtime/src/lib.rs).
// If that changes the runtime will reject our attestation (InvalidChecksHash or
// similar) — safe, but it will stop attesting until this is updated.
// ============================================================

const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const { execFileSync } = require('child_process');
const { ApiPromise, WsProvider } = require('@polkadot/api');
const { Keyring } = require('@polkadot/keyring');
const { cryptoWaitReady, blake2AsU8a } = require('@polkadot/util-crypto');
const { u8aConcat, u8aToHex, hexToU8a } = require('@polkadot/util');

const WS = process.argv[2] || 'ws://127.0.0.1:9944';
const DRY = process.env.DRY_RUN === '1';
const BLS_CATCHUP_BLOCKS = 2000; // how far back to BLS-attest payloads missing an aggregate
const BLS_GEN =
  process.env.BLS_GEN || path.resolve(__dirname, '../target/release/examples/bls_gen');

const log = (...a) => console.log(new Date().toISOString(), ...a);

function loadBlsSeed(addr) {
  const file = process.env.BLS_SEED_FILE || `./.bls-seed-${addr.slice(0, 8)}`;
  if (fs.existsSync(file)) return fs.readFileSync(file, 'utf8').trim();
  const seed = '0x' + crypto.randomBytes(32).toString('hex');
  fs.writeFileSync(file, seed + '\n', { mode: 0o600 });
  log('generated new BLS seed ->', file);
  return seed;
}

// bls_gen prints "<pubkey> <signature>" for (seed, payload_id).
function blsGen(seed, payloadId) {
  const out = execFileSync(BLS_GEN, [seed, payloadId], { encoding: 'utf8' }).trim();
  const [pub, sig] = out.split(/\s+/);
  return { pub, sig };
}

// Extrinsics from one account must go out one at a time: concurrent sends with
// nonce -1 pick the same nonce and the pool rejects them ("Priority is too low").
let sendQueue = Promise.resolve();
function send(api, tx, signer, label) {
  const run = sendQueue.then(() => sendNow(api, tx, signer, label));
  sendQueue = run.catch(() => {});
  return run;
}

// Local nonce counter (re-synced from the chain after any rejection).
let nextNonce = null;
async function allocNonce(api, address) {
  const chain = (await api.rpc.system.accountNextIndex(address)).toNumber();
  nextNonce = Math.max(nextNonce ?? 0, chain);
  return nextNonce++;
}

async function sendNow(api, tx, signer, label) {
  if (DRY) {
    log(`[dry-run] would send ${label}`);
    return;
  }
  const nonce = await allocNonce(api, signer.address);
  return new Promise((resolve) => {
    let entered = false;
    const enter = () => {
      if (!entered) {
        entered = true;
        resolve();
      }
    };
    tx.signAndSend(signer, { nonce }, ({ status, dispatchError }) => {
      enter(); // accepted into the pool: the next tx may take the following nonce
      if (dispatchError) {
        const msg = dispatchError.isModule
          ? (({ section, name }) => `${section}.${name}`)(api.registry.findMetaError(dispatchError.asModule))
          : dispatchError.toString();
        log(`${label} FAILED: ${msg}`);
      } else if (status.isInBlock) {
        log(`${label} in block ${status.asInBlock.toHex().slice(0, 12)}`);
      }
    }).catch((e) => {
      log(`${label} rejected: ${e.message}`);
      nextNonce = null;
      enter();
    });
  });
}

// Mirrors OffchainChecks for `()`: blake2_256(SCALE(creator, nonce, domain,
// payload_hash, expiry, 6 x bool)) with every check passing.
function computeChecksHash(api, p) {
  const enc = api.createType(
    '(AccountId32,u64,u32,[u8;32],BlockNumber,bool,bool,bool,bool,bool,bool)',
    [p.creator, p.nonce, p.domain, p.payloadHash, p.expiry, true, true, true, true, true, true],
  );
  return blake2AsU8a(enc.toU8a(), 256);
}

async function localChecksPass(api, p) {
  const nonceNow = (await api.query.fastlane.nonces(p.creator)).toBigInt();
  if (nonceNow < BigInt(p.nonce) + 1n) return 'nonce not advanced';
  const { data } = await api.query.system.account(p.creator);
  if (data.free.isZero()) return 'creator has zero balance';
  return null;
}

async function main() {
  await cryptoWaitReady();
  if (!process.env.AUTHORITY_URI) throw new Error('AUTHORITY_URI is required');
  const signer = new Keyring({ type: 'sr25519' }).addFromUri(process.env.AUTHORITY_URI);
  const me = signer.address;
  const api = await ApiPromise.create({ provider: new WsProvider(WS) });
  const blsSeed = loadBlsSeed(me);
  log(`attestor for ${me} on ${WS}${DRY ? ' (DRY RUN)' : ''}`);

  const busy = new Set();
  const sent = new Set();

  const ourBlsPub = blsGen(blsSeed, '0x' + '00'.repeat(32)).pub;
  async function blsKeyMatches() {
    const onChain = await api.query.fastlane.authorityBlsKeys(me);
    return onChain.isSome && onChain.unwrap().toHex() === ourBlsPub;
  }

  async function ensureBlsRegistered() {
    if (await blsKeyMatches()) return true;
    log('registering BLS public key on-chain');
    await send(api, api.tx.fastlane.registerBlsKey(ourBlsPub), signer, 'register_bls_key');
    return false; // takes effect next block; attest_bls waits until it matches
  }

  async function handle(payloadId, status) {
    if (busy.has(payloadId)) return;
    busy.add(payloadId);
    try {
      const payload = (await api.query.fastlane.payloads(payloadId)).unwrap().toJSON();
      const header = await api.rpc.chain.getHeader();
      if (header.number.toNumber() >= payload.expiry) return;

      const jobs = [];

      // sr25519 attestation (only while still Submitted)
      const attestations = await api.query.fastlane.attestations(payloadId);
      if (status === 'Submitted' && !sent.has('s:' + payloadId) &&
          !attestations.some((r) => r.authority.toString() === me)) {
        sent.add('s:' + payloadId);
        const checks = computeChecksHash(api, payload);
        const msg = blake2AsU8a(
          u8aConcat(
            hexToU8a(payloadId),
            api.createType('BlockNumber', payload.expiry).toU8a(),
            checks,
          ),
          256,
        );
        const sig = u8aToHex(signer.sign(msg));
        jobs.push(send(
          api,
          api.tx.fastlane.attest(payloadId, me, sig, u8aToHex(checks)),
          signer,
          `attest ${payloadId.slice(0, 10)}`,
        ));
      }

      // BLS attestation, in parallel (only once our key is the registered one;
      // the runtime accepts it any time before expiry, even after PreConsensed)
      if (!sent.has('b:' + payloadId) && (await blsKeyMatches())) {
        const done = (await api.query.fastlane.blsAttestations(payloadId)).some(
          ([a]) => a.toString() === me,
        );
        if (!done) {
          sent.add('b:' + payloadId);
          const { sig } = blsGen(blsSeed, payloadId);
          jobs.push(send(
            api,
            api.tx.fastlane.attestBls(payloadId, sig),
            signer,
            `attest_bls ${payloadId.slice(0, 10)}`,
          ));
        }
      }
      await Promise.all(jobs);
    } catch (e) {
      log(`error on ${payloadId.slice(0, 10)}: ${e.message}`);
    } finally {
      busy.delete(payloadId);
    }
  }

  // Register our BLS key up front so it is live before the first payload.
  if ((await api.query.fastlane.authorities()).map((a) => a.toString()).includes(me)) {
    await ensureBlsRegistered();
  }

  // Each new block: attest to everything still Submitted.
  await api.rpc.chain.subscribeNewHeads(async () => {
    const auth = (await api.query.fastlane.authorities()).map((a) => a.toString());
    if (!auth.includes(me)) return;
    const entries = await api.query.fastlane.statuses.entries();
    const best = (await api.rpc.chain.getHeader()).number.toNumber();
    for (const [key, status] of entries) {
      const id = key.args[0].toHex();
      const st = status.toString();
      if (st === 'Submitted') {
        handle(id, st);
      } else if (st !== 'Expired' && !(await api.query.fastlane.blsAggregate(id)).isSome) {
        // Late BLS-only catch-up, bounded to recent payloads.
        const p = (await api.query.fastlane.payloads(id)).unwrap();
        if (best - p.createdAt.toNumber() < BLS_CATCHUP_BLOCKS) handle(id, st);
      }
    }
  });
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
