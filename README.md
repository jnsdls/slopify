# slopify

A macOS menu-bar player for one Premium Spotify account. Click the icon, pick a playlist or Liked Songs, and audio plays out of slopify itself. Play, pause, next, volume. That is the whole app.

What it adds over Spotify's own client is the menu bar. One click from any app starts the playlist you always start, and Spotify desktop does not need to be running, or installed. The player is Spotify's Web Playback SDK inside a castLabs Electron build with Widevine, so playback is licensed the same way the browser player is. The app is not published anywhere; the owner builds it locally and copies it to /Applications.

The design and every verified detail are in [docs/spec/v1.md](docs/spec/v1.md). Vocabulary (Listener, Source, Player, Dropdown, Resume Point) is in [CONTEXT.md](CONTEXT.md).

## Requirements

- A Spotify Premium account. The SDK refuses Free accounts and slopify shows a sign-in screen saying so.
- Your account on the app's allowlist. The Spotify client is in Development Mode, which caps it at five users, so only the owner and a few people can sign in. Ask the owner.
- An Apple silicon Mac. The build is arm64 only.
- Spotify desktop closed while slopify plays. macOS gives media keys and the Now Playing widget to whichever app last reported playing. Spotify desktop mirrors the Connect session as soon as it is open, so it takes Now Playing over and F8 and AirPods presses go through it instead of slopify. Playback keeps working either way, but the widget shows Spotify, not slopify. Quit Spotify desktop, or do not install it.

## Install

Building needs pnpm, Python 3 with the `castlabs-evs` package, and the owner's castLabs EVS credentials in the macOS Keychain.

```zsh
pip3 install castlabs-evs
security add-generic-password -a "$USER" -s castlabs-evs-account-name -U -T /usr/bin/security -w
security add-generic-password -a "$USER" -s castlabs-evs-password -U -T /usr/bin/security -w
```

Each `security` command prompts for the value. The `postinstall` script reads these items to VMP-sign the dev Electron binary; without that signature Spotify's licence server returns 500 and nothing plays.

```zsh
pnpm install
pnpm dev        # runs the app from source
pnpm package    # builds dist/mac-arm64/slopify.app
cp -R dist/mac-arm64/slopify.app /Applications/
```

`pnpm package` refreshes the EVS token, builds with electron-vite, packages with electron-builder, VMP-signs, then ad-hoc codesigns. Electron's fuses stay stock, because the EVS server refuses to sign a binary with flipped fuses. One consequence: if `ELECTRON_RUN_AS_NODE` is set in the environment that launches the app, the binary starts as plain Node and exits. Finder and the login item never set it; a shell that does needs `env -u ELECTRON_RUN_AS_NODE open /Applications/slopify.app`. The EVS token expires monthly, and a stale token is the usual reason a build fails. There is no DMG, no notarisation and no updater. Rebuild and copy again.

`pnpm check` runs the type check, ESLint and the unit tests.

## Signing in

First launch opens Spotify's login page in your browser. Approve the app there and slopify picks up the callback on `http://127.0.0.1:8888`. The refresh token goes into your login Keychain as `slopify-spotify-refresh-token`; nothing else is stored about your account.

There is no sign-out and no account switcher. To sign in as a different account, quit slopify and delete the Keychain item, then launch again.

```zsh
security delete-generic-password -s slopify-spotify-refresh-token
```

If Spotify invalidates the token (you revoked the app at spotify.com/account/apps, or changed your password), slopify shows the sign-in screen with a dot on the tray icon, and signing in again fixes it.

## Before calling a build done

The manual checklist is in [docs/spec/v1.md](docs/spec/v1.md) under "Testing and done". It covers a fresh sign-in, each Source type, resume after relaunch, media keys and Now Playing, take-back from a phone, sleep and wake, and the signature checks on the packaged app. Run it on the built `.app`, not on `pnpm dev`.
