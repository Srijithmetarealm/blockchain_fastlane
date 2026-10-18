// ============================================================
// scripts/upgrade-runtime.js
// Forkless runtime upgrade: push the new Wasm on-chain via
// sudo(system.setCode(...)), signed by Alice (the sudo key).
//
// Usage:
//   node scripts/upgrade-runtime.js <ws-endpoint> <path-to-compact-compressed.wasm>
// Defaults: ws://127.0.0.1:9944 and the standard wbuild output path.
// Run from a dir where @polkadot/api is installed (e.g. ../fastlane-api).
// ============================================================

const fs = require('fs');
const { ApiPromise, WsProvider } = require('@polkadot/api');
const { Keyring } = require('@polkadot/keyring');
const { cryptoWaitReady } = require('@polkadot/util-crypto');

const WS = process.argv[2] || 'ws://127.0.0.1:9944';
const WASM_PATH =
  process.argv[3] ||
  '/home/srijith/my-blockchain/target/release/wbuild/solochain-template-runtime/solochain_template_runtime.compact.compressed.wasm';

async function main() {
  await cryptoWaitReady();

  const code = fs.readFileSync(WASM_PATH);
  const codeHex = '0x' + code.toString('hex');
  console.log(`Wasm: ${WASM_PATH} (${(code.length / 1024).toFixed(1)} KiB)`);

  const api = await ApiPromise.create({ provider: new WsProvider(WS) });
  const before = (await api.query.system.lastRuntimeUpgrade()).toJSON();
  console.log('Connected. Current runtime version:', api.runtimeVersion.specVersion.toString());
  console.log('lastRuntimeUpgrade (before):', JSON.stringify(before));

  const keyring = new Keyring({ type: 'sr25519' });
  const alice = keyring.addFromUri('//Alice');

  // system.setCode carries a max-block weight, so it must go through
  // sudoUncheckedWeight to bypass the weight check for this one call.
  const setCode = api.tx.system.setCode(codeHex);
  const call = api.tx.sudo.sudoUncheckedWeight(setCode, {
    refTime: 1,
    proofSize: 1,
  });

  console.log('Submitting sudo(setCode) as Alice…');
  await new Promise((resolve, reject) => {
    let unsub;
    call
      .signAndSend(alice, ({ status, dispatchError, events }) => {
        if (dispatchError) {
          let msg = dispatchError.toString();
          if (dispatchError.isModule) {
            const d = api.registry.findMetaError(dispatchError.asModule);
            msg = `${d.section}.${d.name}`;
          }
          if (unsub) unsub();
          reject(new Error(msg));
          return;
        }
        if (status.isInBlock) {
          console.log(`Included in block ${status.asInBlock.toString()}`);
          for (const { event } of events) {
            console.log(`  event: ${event.section}.${event.method}`);
          }
          if (unsub) unsub();
          resolve();
        }
      })
      .then((u) => (unsub = u))
      .catch(reject);
  });

  // Give the chain a couple of blocks to apply the upgrade, then re-read.
  await new Promise((r) => setTimeout(r, 12000));
  const ver = await api.rpc.state.getRuntimeVersion();
  console.log('New runtime specVersion:', ver.specVersion.toString());
  const hasCommit = !!api.tx.fastlane && !!api.tx.fastlane.commitInclusion;
  console.log('fastlane.commitInclusion present in metadata (pre-reconnect):', hasCommit);

  await api.disconnect();
  console.log('Done.');
}

main().catch((e) => {
  console.error('Upgrade failed:', e.message || e);
  process.exit(1);
});
