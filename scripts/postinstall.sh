#!/bin/zsh
# Fetch the castLabs Electron binary if pnpm skipped its install script, then VMP-sign it so
# Spotify's licence server accepts the dev build (spec: Development). Signing is skipped when
# the EVS Keychain items are absent, so a plain install still works.
set -e
cd "$(dirname "$0")/.."
[ -f node_modules/electron/install.js ] || exit 0
if [ ! -d node_modules/electron/dist ]; then
  (cd node_modules/electron && env -u ELECTRON_RUN_AS_NODE node install.js)
fi
security find-generic-password -s castlabs-evs-account-name -w >/dev/null 2>&1 || { echo "postinstall: no castlabs-evs-* Keychain items, skipping VMP signing"; exit 0; }
python3 -m castlabs_evs.vmp sign-pkg node_modules/electron/dist
python3 -m castlabs_evs.vmp verify-pkg node_modules/electron/dist
