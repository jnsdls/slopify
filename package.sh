#!/bin/zsh
# Build dist/slopify.app from the Rust app. No VMP step: WKWebView brings its own FairPlay
# (docs/adr/0002), so an ad-hoc signature is all the bundle needs.
set -e
cd "$(dirname "$0")"

# 1. The Spotify client id is compiled in. CI passes a placeholder; locally it comes from .env.local.
if [[ -z $SLOPIFY_SPOTIFY_CLIENT_ID && -f .env.local ]]; then
  SLOPIFY_SPOTIFY_CLIENT_ID=$(sed -n 's/^SLOPIFY_SPOTIFY_CLIENT_ID=//p' .env.local)
fi
if [[ -z $SLOPIFY_SPOTIFY_CLIENT_ID ]]; then
  echo "set SLOPIFY_SPOTIFY_CLIENT_ID in .env.local" >&2
  exit 1
fi
export SLOPIFY_SPOTIFY_CLIENT_ID

# 2. Release build. 13.0 is the floor SMAppService sets for the login item.
MIN_MACOS=13.0
MACOSX_DEPLOYMENT_TARGET=$MIN_MACOS cargo build --release -p slopify
VERSION=${$(cargo pkgid -p slopify)##*@}

# 3. Assemble the bundle.
APP=dist/slopify.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/slopify "$APP/Contents/MacOS/slopify"
cp build/icon.icns "$APP/Contents/Resources/icon.icns"
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>slopify</string>
  <key>CFBundleExecutable</key><string>slopify</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundleIdentifier</key><string>gg.nebula.slopify</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>slopify</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.music</string>
  <key>LSMinimumSystemVersion</key><string>$MIN_MACOS</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
plutil -lint "$APP/Contents/Info.plist"

# 4. Ad-hoc codesign, then verify.
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"
echo "built $APP"
