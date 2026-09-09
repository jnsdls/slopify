// electron-builder afterPack hook: VMP-sign the packaged Electron with castLabs EVS, then verify.
// CommonJS because electron-builder loads hooks with require() first; the package is "type": "module".
const { execFileSync } = require('node:child_process');

const evs = (cmd, dir) =>
  execFileSync('python3', ['-m', 'castlabs_evs.vmp', '-n', cmd, dir], { stdio: 'inherit' });

exports.default = async ({ appOutDir }) => {
  evs('sign-pkg', appOutDir);
  evs('verify-pkg', appOutDir);
};
