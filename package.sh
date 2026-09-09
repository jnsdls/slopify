#!/bin/zsh
# Build dist/mac-arm64/slopify.app. Steps follow docs/spec/v1.md "Packaging and signing".
set -e
cd "$(dirname "$0")"

# 1. Refresh the EVS account token (expires monthly; the usual reason a build fails).
python3 -m castlabs_evs.account -n refresh \
  -A "$(security find-generic-password -s castlabs-evs-account-name -w)" \
  -P "$(security find-generic-password -s castlabs-evs-password -w)"

# 2. Compile main, preload and renderer into out/.
pnpm exec electron-vite build

# 3. Package. afterPack.cjs VMP-signs the bundle; electron-builder 26.15.3 flips `electronFuses`
#    after the afterPack hook, so the order is VMP, fuses, then the ad-hoc codesign in step 4.
#    Spec, "Packaging and signing", paragraph "VMP before codesign, always": if verify-pkg below
#    fails, swap to fuses-before-VMP by dropping `electronFuses` from package.json and calling
#    `context.packager.addElectronFuses(context, {...})` at the top of afterPack.cjs.
env -u ELECTRON_RUN_AS_NODE pnpm exec electron-builder --mac --dir

# 4. Ad-hoc codesign after VMP, then verify both signatures.
APP=dist/mac-arm64/slopify.app
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"
python3 -m castlabs_evs.vmp -n verify-pkg dist/mac-arm64
echo "built $APP"
