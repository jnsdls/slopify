// electron-builder afterPack hook: VMP-sign the packaged Electron with castLabs EVS.
// Runs before electron-builder's own codesign step, which is the order VMP needs.
const { execFileSync } = require('child_process');
exports.default = async ({ appOutDir }) => {
  execFileSync('python3', ['-m', 'castlabs_evs.vmp', '-n', 'sign-pkg', appOutDir], { stdio: 'inherit' });
  execFileSync('python3', ['-m', 'castlabs_evs.vmp', '-n', 'verify-pkg', appOutDir], { stdio: 'inherit' });
};
