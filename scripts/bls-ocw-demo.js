// ============================================================
// scripts/bls-ocw-demo.js
// Proof that the offchain worker auto-produces a BLS aggregate.
// Submit a payload, then poll until BlsAggregate appears — WITHOUT any
// script submitting attest_bls. The aggregate can only appear if each
// authority's OCW signed and submitted attest_bls_unsigned on its own.
//
//   NODE_PATH=.../@polkadot node scripts/bls-ocw-demo.js ws://localhost:9955
// ============================================================

const crypto = require('crypto');
const { ApiPromise, WsProvider } = require('@polkadot/api');
const { Keyring } = require('@polkadot/keyring');
const { cryptoWaitReady } = require('@polkadot/util-crypto');

const WS = process.argv[2] || 'ws://localhost:9955';

async function main() {
  await cryptoWaitReady();
  const api = await ApiPromise.create({ provider: new WsProvider(WS) });
  const alice = new Keyring({ type: 'sr25519' }).addFromUri('//Alice');

  // Genesis should have auto-registered Alice's BLS key.
  const blsKey = await api.query.fastlane.authorityBlsKeys(alice.address);
  console.log('Alice BLS key present at genesis:', blsKey.isSome ? 'YES' : 'NO');

  const nonce = (await api.query.fastlane.nonces(alice.address)).toNumber();
  const now = (await api.rpc.chain.getHeader()).number.toNumber();
  const hash = '0x' + crypto.randomBytes(32).toString('hex');
  console.log(`Submitting payload (nonce=${nonce})...`);

  let pid;
  await new Promise((resolve, reject) => {
    api.tx.fastlane.submit(nonce, now + 100000, 1, hash).signAndSend(alice, ({ status, dispatchError, events }) => {
      if (dispatchError) return reject(new Error(dispatchError.toString()));
      if (status.isInBlock) {
        const ev = events.find(({ event }) => event.section === 'fastlane' && event.method === 'Submitted');
        pid = ev.event.data[0].toString();
        resolve();
      }
    }).catch(reject);
  });
  console.log('payloadId:', pid);
  console.log('Waiting for the OCW to auto-produce a BLS aggregate (no script attests)...');

  for (let i = 0; i < 40; i++) {
    const agg = await api.query.fastlane.blsAggregate(pid);
    if (agg.isSome) {
      const hex = agg.unwrap().toHex();
      const recs = await api.query.fastlane.blsAttestations(pid);
      console.log(`\n✅ OCW auto-produced BlsAggregate after ~${i}s`);
      console.log(`   ${recs.length} BLS attestation(s) collected -> one ${(hex.length - 2) / 2}-byte aggregate`);
      console.log('   aggregate =', hex);
      await api.disconnect();
      process.exit(0);
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  console.log('\n❌ No aggregate after 40s — OCW may not be running/keyed.');
  process.exit(1);
}

main().catch((e) => { console.error('failed:', e.message); process.exit(1); });
