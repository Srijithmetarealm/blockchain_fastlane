// ============================================================
// scripts/bls-live-demo.js
// Live proof of §14 real BLS aggregation against the running chain:
//   1. submit a payload
//   2. each authority registers its BLS key and submits attest_bls (its BLS
//      signature is verified ON-CHAIN)
//   3. at threshold the pallet stores ONE aggregate signature — read it back
//
// BLS material is produced by the pallet-matching generator:
//   target/release/examples/bls_gen <seed_hex> <payload_id_hex>
//
// Run with @polkadot/api available, e.g.:
//   NODE_PATH=/home/srijith/fastlane-api/node_modules node scripts/bls-live-demo.js
// ============================================================

const { execFileSync } = require('child_process');
const crypto = require('crypto');
const { ApiPromise, WsProvider } = require('@polkadot/api');
const { Keyring } = require('@polkadot/keyring');
const { cryptoWaitReady } = require('@polkadot/util-crypto');

const WS = process.argv[2] || 'ws://localhost:9944';
const BLS_GEN =
  '/home/srijith/my-blockchain/target/release/examples/bls_gen';

function genBls(seedHex, payloadIdHex) {
  const out = execFileSync(BLS_GEN, [seedHex, payloadIdHex]).toString().trim();
  const [pubkey, sig] = out.split(' ');
  return { pubkey, sig };
}

function send(api, tx, signer) {
  return new Promise((resolve, reject) => {
    tx.signAndSend(signer, ({ status, dispatchError }) => {
      if (dispatchError) {
        let msg = dispatchError.toString();
        if (dispatchError.isModule) {
          const d = api.registry.findMetaError(dispatchError.asModule);
          msg = `${d.section}.${d.name}`;
        }
        return reject(new Error(msg));
      }
      if (status.isInBlock) resolve();
    }).catch(reject);
  });
}

async function main() {
  await cryptoWaitReady();
  const api = await ApiPromise.create({ provider: new WsProvider(WS) });
  const kr = new Keyring({ type: 'sr25519' });

  const authorities = [
    { name: 'Alice', signer: kr.addFromUri('//Alice'), seed: '0x' + '01'.repeat(32) },
    { name: 'Bob', signer: kr.addFromUri('//Bob'), seed: '0x' + '02'.repeat(32) },
    { name: 'Charlie', signer: kr.addFromUri('//Charlie'), seed: '0x' + '03'.repeat(32) },
    { name: 'Dave', signer: kr.addFromUri('//Dave'), seed: '0x' + '04'.repeat(32) },
  ];
  const creator = kr.addFromUri('//Alice');

  // 1) Submit a payload with a far-future expiry.
  const nonce = (await api.query.fastlane.nonces(creator.address)).toNumber();
  const now = (await api.rpc.chain.getHeader()).number.toNumber();
  const expiry = now + 100000;
  const hash = '0x' + crypto.randomBytes(32).toString('hex');
  console.log(`Submitting payload: creator=Alice nonce=${nonce} expiry=${expiry}`);

  let payloadId;
  await new Promise((resolve, reject) => {
    api.tx.fastlane.submit(nonce, expiry, 1, hash).signAndSend(creator, ({ status, dispatchError, events }) => {
      if (dispatchError) return reject(new Error(dispatchError.toString()));
      if (status.isInBlock) {
        const ev = events.find(({ event }) => event.section === 'fastlane' && event.method === 'Submitted');
        payloadId = ev.event.data[0].toString();
        resolve();
      }
    }).catch(reject);
  });
  console.log('payloadId:', payloadId, '\n');

  // 2) Each authority registers its BLS key and BLS-attests the payload.
  for (const a of authorities) {
    const { pubkey, sig } = genBls(a.seed, payloadId);
    await send(api, api.tx.fastlane.registerBlsKey(pubkey), a.signer);
    await send(api, api.tx.fastlane.attestBls(payloadId, sig), a.signer);
    console.log(`  ${a.name}: BLS key registered + attest_bls accepted (signature verified ON-CHAIN)`);
  }

  // 3) Read back the single aggregate signature.
  const agg = await api.query.fastlane.blsAggregate(payloadId);
  if (agg.isSome) {
    const hex = agg.unwrap().toHex();
    const len = (hex.length - 2) / 2;
    console.log(`\n✅ On-chain BlsAggregate for ${authorities.length} signers: ${len} bytes`);
    console.log('   aggregate =', hex);
  } else {
    console.log('\n❌ No aggregate stored (threshold not reached?)');
  }
  await api.disconnect();
  process.exit(0);
}

main().catch((e) => {
  console.error('demo failed:', e.message || e);
  process.exit(1);
});
