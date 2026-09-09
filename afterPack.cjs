// electron-builder afterPack hook: VMP-sign the packaged Electron with castLabs EVS, then verify.
// No fuse flip: EVS refuses to sign a binary whose hash it does not know ("Binary signature denied",
// 2026-09-09), and flipping after VMP invalidates the signature. See package.sh step 3.
// CommonJS because electron-builder loads hooks with require() first; the package is "type": "module".
const { execFileSync } = require('node:child_process');

const evs = (cmd, dir) =>
  execFileSync('python3', ['-m', 'castlabs_evs.vmp', '-n', cmd, dir], { stdio: 'inherit' });

exports.default = async ({ appOutDir }) => {
  evs('sign-pkg', appOutDir);
  evs('verify-pkg', appOutDir);
};
