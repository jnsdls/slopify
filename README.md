# slopify

A macOS menu-bar player for one Premium Spotify account. Click the icon, pick a playlist or Liked Songs, and audio plays out of slopify itself. Play, pause, next, volume. That is the whole app.

What it adds over Spotify's own client is the menu bar. One click from any app starts the playlist you always start, and Spotify desktop does not need to be running, or installed. The player is Spotify's Web Playback SDK inside a castLabs Electron build with Widevine, so playback is licensed the same way the browser player is.

There is no download. You build it from source against a Spotify developer app you register yourself, and the build only signs in accounts you put on that app's allowlist. Spotify's Development Mode caps an app at five users, which is fine for you and a few friends.

The design and every verified detail are in [docs/spec/v1.md](docs/spec/v1.md). Vocabulary (Listener, Source, Player, Dropdown, Resume Point) is in [CONTEXT.md](CONTEXT.md).

## Requirements

- An Apple silicon Mac. The build is arm64 only.
- A Spotify Premium account. The SDK refuses Free accounts and slopify shows a sign-in screen saying so.
- Node 24 and pnpm 10, plus Python 3 for the castLabs signing tool.
- Spotify desktop closed while slopify plays. macOS gives media keys and the Now Playing widget to whichever app last reported playing. Spotify desktop mirrors the Connect session as soon as it is open, so it takes Now Playing over and F8 and AirPods presses go through it instead of slopify. Playback keeps working either way, but the widget shows Spotify, not slopify. Quit Spotify desktop, or do not install it.

## Quick start

1. Create a Spotify app at [developer.spotify.com/dashboard](https://developer.spotify.com/dashboard). Add `http://127.0.0.1:8888/callback` as a redirect URI and tick "Web Playback SDK" and "Web API" under APIs used. Copy the client id. Under "User management", add the email of every Spotify account that will sign in, including your own.

2. Create a free [castLabs EVS](https://github.com/castlabs/electron-releases/wiki/EVS) account and store its credentials in your login Keychain. Without a VMP signature from EVS, Spotify's licence server returns 500 and nothing plays.

   ```zsh
   pip3 install castlabs-evs
   python3 -m castlabs_evs.account signup
   security add-generic-password -a "$USER" -s castlabs-evs-account-name -U -T /usr/bin/security -w
   security add-generic-password -a "$USER" -s castlabs-evs-password -U -T /usr/bin/security -w
   ```

   Each `security` command prompts for the value.

3. Point the build at your Spotify app.

   ```zsh
   git clone https://github.com/jnsdls/slopify.git && cd slopify
   cp .env.example .env.local
   # paste the client id after MAIN_VITE_SPOTIFY_CLIENT_ID=
   ```

4. Install and run.

   ```zsh
   pnpm install    # also VMP-signs the dev Electron binary
   pnpm dev        # runs the app from source
   ```

5. Build the app and put it in Applications.

   ```zsh
   pnpm package
   cp -R dist/mac-arm64/slopify.app /Applications/
   ```

Each build is ad-hoc signed, so macOS sees every new build as a new app. The first time a build plays audio, macOS may ask for microphone access once. slopify never captures audio, so "Don't Allow" is the right answer, and that build will not ask again.

`pnpm package` refreshes the EVS token, builds with electron-vite, packages with electron-builder, VMP-signs, then ad-hoc codesigns. Electron's fuses stay stock, because the EVS server refuses to sign a binary with flipped fuses. One consequence: if `ELECTRON_RUN_AS_NODE` is set in the environment that launches the app, the binary starts as plain Node and exits. Finder and the login item never set it; a shell that does needs `env -u ELECTRON_RUN_AS_NODE open /Applications/slopify.app`. The EVS token expires monthly, and a stale token is the usual reason a build fails. There is no DMG, no notarisation and no updater. Rebuild and copy again.

## Signing in

First launch opens Spotify's login page in your browser. Approve the app there and slopify picks up the callback on `http://127.0.0.1:8888`. The refresh token goes into your login Keychain as `slopify-spotify-refresh-token`; nothing else is stored about your account.

There is no sign-out and no account switcher. To sign in as a different account, quit slopify and delete the Keychain item, then launch again.

```zsh
security delete-generic-password -s slopify-spotify-refresh-token
```

If Spotify invalidates the token (you revoked the app at spotify.com/account/apps, or changed your password), slopify shows the sign-in screen with a dot on the tray icon, and signing in again fixes it.

## Contributing

Bug reports and pull requests are welcome. [CONTRIBUTING.md](CONTRIBUTING.md) covers the checks a PR has to pass and what is out of scope. `pnpm check` runs the type check, ESLint and the unit tests.

Before calling a build done, run the manual checklist in [docs/spec/v1.md](docs/spec/v1.md) under "Testing and done". It covers a fresh sign-in, each Source type, resume after relaunch, media keys and Now Playing, take-back from a phone, sleep and wake, and the signature checks on the packaged app. Run it on the built `.app`, not on `pnpm dev`.

## License

[MIT](LICENSE).
