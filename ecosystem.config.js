// ============================================================
// PM2 process definitions for the FastLane stack.
//
// Replaces the ad-hoc `fastlane-node` entry that ran the node as
// `--dev --rpc-port 9944`. That node used a temporary keystore with no
// `fast` keys, so its offchain worker bailed at the empty-key check and
// no payload could ever be attested — while still holding port 9944, which
// nginx proxies to and the dashboard reads. Supervising the real validator
// scripts instead keeps the bonded authorities on their expected ports
// across reboots.
//
// Ports must match fastlane-api/src/config/index.js NODES[].rpcUrl.
//
// Usage:
//   pm2 delete fastlane-node          # remove the old --dev entry
//   pm2 start ecosystem.config.js
//   pm2 save                          # persist across reboot
// ============================================================

const CWD = '/home/srijith/my-blockchain';

// Alice must come up first: she is the bootnode the others dial via the
// fixed --node-key in run-alice.sh.
const validator = (name, script, extra = {}) => ({
  name: `fastlane-${name}`,
  script: `${CWD}/scripts/${script}`,
  interpreter: 'bash',
  cwd: CWD,
  autorestart: true,
  // These nodes open multi-GB RocksDB stores (dave ~13G) and can take a
  // minute before binding their RPC port. Give them room before PM2
  // considers the start a success, and back off rather than thrash.
  min_uptime: '90s',
  max_restarts: 5,
  restart_delay: 10000,
  kill_timeout: 30000,
  out_file: `${CWD}/data/${name}.log`,
  error_file: `${CWD}/data/${name}.log`,
  merge_logs: true,
  time: false,
  ...extra,
});

module.exports = {
  apps: [
    validator('alice', 'run-alice.sh'),
    // Bootstrap delay: the others use Alice as their bootnode.
    validator('bob', 'run-bob.sh'),
    validator('charlie', 'run-charlie.sh'),
    validator('dave', 'run-dave.sh'),
    validator('eve', 'run-eve.sh'),
  ],
};
