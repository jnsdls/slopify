#!/bin/zsh
# Build slopify-spike.app: electron-builder packages the castLabs binary, afterPack.js
# VMP-signs it with EVS, then this script ad-hoc codesigns the bundle. Order matters:
# VMP first, codesign second. Verified 2026-09-09 that codesign leaves the VMP signature valid.
set -e
cd "$(dirname "$0")"
python3 -m castlabs_evs.account -n refresh -A "$(security find-generic-password -s castlabs-evs-account-name -w)" \
  -P "$(security find-generic-password -s castlabs-evs-password -w)"
env -u ELECTRON_RUN_AS_NODE pnpm exec electron-builder --mac --dir
APP=out/mac-arm64/slopify-spike.app
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"
python3 -m castlabs_evs.vmp -n verify-pkg out/mac-arm64
echo "built $APP"
