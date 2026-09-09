#!/bin/zsh
# EVS-sign the castLabs Electron binary, then run the spike.
# Needs an EVS account: python3 -m castlabs_evs.account signup (interactive, emails a code).
set -e
cd "$(dirname "$0")"
python3 -m castlabs_evs.vmp sign-pkg node_modules/electron/dist
python3 -m castlabs_evs.vmp verify-pkg node_modules/electron/dist
rm -f spike.log
env -u ELECTRON_RUN_AS_NODE pnpm exec electron . 2>&1 | grep -vE 'CoreText|font'
