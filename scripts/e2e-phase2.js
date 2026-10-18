// End-to-end check of the §14 features on the LIVE upgraded chain.
// submit -> (OCW attests) -> PreConsensed -> aggregate -> commit_inclusion ->
// finalize -> confirm signatures pruned but aggregate retained.
const { ApiPromise, WsProvider } = require('@polkadot/api');
const { Keyring } = require('@polkadot/keyring');
const { cryptoWaitReady, blake2AsU8a } = require('@polkadot/util-crypto');
const { u8aConcat, hexToU8a, u8aToHex, stringToU8a } = require('@polkadot/util');

const WS = process.argv[2] || 'ws://127.0.0.1:9944';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function sendWait(api, tx, signer, label) {
  return new Promise((resolve, reject) => {
    tx.signAndSend(signer, ({ status, dispatchError }) => {
      if (dispatchError) {
        let m = dispatchError.toString();
        if (dispatchError.isModule) { const d = api.registry.findMetaError(dispatchError.asModule); m = `${d.section}.${d.name}`; }
        reject(new Error(`${label}: ${m}`));
      } else if (status.isInBlock) {
        console.log(`  ${label} in block ${status.asInBlock.toString().slice(0, 12)}…`);
        resolve();
      }
    }).catch(reject);
  });
}

async function main() {
  await cryptoWaitReady();
  const api = await ApiPromise.create({ provider: new WsProvider(WS) });
  const kr = new Keyring({ type: 'sr25519' });
  const alice = kr.addFromUri('//Alice');

  console.log('runtime specVersion:', api.runtimeVersion.specVersion.toString());

  const now = (await api.query.system.number()).toNumber();
  const nonce = (await api.query.fastlane.nonces(alice.address)).toNumber();
  const domain = 1;
  const expiry = now + 500;
  const hash = u8aToHex(blake2AsU8a(stringToU8a('phase2-e2e-' + Date.now()), 256));
  const pid = u8aToHex(blake2AsU8a(
    api.registry.createType('(AccountId, u64, u32, [u8;32])', [alice.address, nonce, domain, hash]).toU8a(), 256));

  console.log('submitting payload', pid.slice(0, 14) + '…', `(nonce=${nonce}, expiry=${expiry})`);
  await sendWait(api, api.tx.fastlane.submit(nonce, expiry, domain, hash), alice, 'submit');

  // Wait for the OCW authorities to attest and reach threshold.
  let status = '';
  for (let i = 0; i < 20; i++) {
    await sleep(3000);
    status = (await api.query.fastlane.statuses(pid)).toString();
    const agg = await api.query.fastlane.aggregatedAttestations(pid);
    console.log(`  t+${(i + 1) * 3}s status=${status} aggregate=${agg.isSome ? agg.unwrap().count.toString() + ' sigs' : 'none'}`);
    if (status === 'PreConsensed' && agg.isSome) break;
  }

  const agg = await api.query.fastlane.aggregatedAttestations(pid);
  if (agg.isNone) throw new Error('no aggregate after waiting — OCW did not reach threshold');
  const a = agg.unwrap();
  console.log(`AGGREGATE ok: count=${a.count} bitmap=${a.signerBitmap} digest=${a.digest.toString().slice(0, 14)}…`);

  // Inclusion commitment (Alice is an authority).
  const deadline = (await api.query.system.number()).toNumber() + 50;
  const msg = u8aConcat(hexToU8a(pid), stringToU8a('INCLUDE'), api.registry.createType('BlockNumber', deadline).toU8a());
  const sig = alice.sign(blake2AsU8a(msg, 256));
  await sendWait(api, api.tx.fastlane.commitInclusion(pid, deadline, u8aToHex(sig)), alice, 'commit_inclusion');
  const incs = await api.query.fastlane.inclusionCommitments(pid);
  console.log(`INCLUSION ok: ${incs.length} commitment(s), deadline block ${deadline}`);

  // Finalize and confirm pruning.
  await sendWait(api, api.tx.fastlane.finalize(pid), alice, 'finalize');
  const finalStatus = (await api.query.fastlane.statuses(pid)).toString();
  const attsAfter = await api.query.fastlane.attestations(pid);
  const aggAfter = await api.query.fastlane.aggregatedAttestations(pid);
  console.log(`FINALIZE ok: status=${finalStatus} rawSigs=${attsAfter.length} aggregateRetained=${aggAfter.isSome}`);

  console.log(attsAfter.length === 0 && aggAfter.isSome && finalStatus === 'Finalised'
    ? '\n✅ E2E PASS: aggregation + inclusion commitment + prune-on-finalise all working on live chain.'
    : '\n❌ E2E unexpected state.');

  await api.disconnect();
}
main().catch((e) => { console.error('E2E failed:', e.message || e); process.exit(1); });
