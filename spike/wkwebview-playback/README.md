# WKWebView playback spike

Throwaway spike for [#22](https://github.com/jnsdls/slopify/issues/22). It asks whether Spotify's Web Playback SDK plays inside a plain WKWebView on macOS, through `wry`, with Apple's FairPlay CDM and no Widevine or castLabs.

It does. The SDK picks FairPlay on its own, Spotify's FairPlay licence server returns 200, and the track plays past 30 s in a hidden window.

## Run

```sh
cargo run --release -- --hidden
```

Flags:

- `--hidden` keeps the window off screen and switches the app to the accessory activation policy, the way a menu bar app would run.
- `--origin custom|localhost` serves the page from the `spike://localhost/` custom scheme (default) or from a throwaway HTTP server on `127.0.0.1`.
- `--no-autoplay` leaves `mediaTypesRequiringUserActionForPlayback` at the WKWebView default instead of setting it to none.
- `--throttling disabled` turns off WebKit background throttling. No run needed it.

The binary refreshes the access token once from the Keychain item the real app uses (`slopify-spotify-refresh-token`). If Spotify rotates the refresh token, the binary writes the new one back through `security -i` on stdin and reads it back to check, before anything else runs. Spotify desktop and slopify must be closed.

After the SDK fires `ready` it sends `PUT /v1/me/player/play` with one track, plays for about 33 s, and samples memory and `media-control get`. If Now Playing belongs to this process tree it sends `media-control toggle-play-pause` twice. Then it pauses through the SDK, disconnects, and prints `RESULT: PLAYED <n>s` or `RESULT: FAILED <reason>`. Everything also goes to `spike.log`, which is gitignored. The log holds no tokens: the injected script redacts URLs and never logs headers or bodies.

`src/inject.js` runs at document start in every frame, including the SDK's cross-origin iframe. It forwards console output, fetch/XHR URL and status, EME calls, and `HTMLMediaElement.play()` results to Rust over the wry IPC handler.

## Findings

macOS 15.7.3, wry 0.57, tao 0.37, SDK build `harmony` 4.67.0. Four runs, all `RESULT: PLAYED 36s`: visible and hidden with the custom scheme, hidden with autoplay off, hidden from `127.0.0.1`.

**Key system.** The SDK never calls `navigator.requestMediaKeySystemAccess`. It uses the legacy prefixed `WebKitMediaKeys('com.apple.fps.1_0')` inside its iframe at `https://sdk.scdn.co/embedded/index.html`.

**Licence flow.** All three requests come from the iframe:

```
GET  https://spclient.wg.spotify.com/fairplay-license/v1/application-certificate      200
GET  https://api.spotify.com/v1/melody/v1/license_url?keysystem=com.apple.fps.1_0&...   200
POST https://api.spotify.com/v1/fairplay-license/v1/audio/license?assetId=hex         200
```

About 0.2 s after the licence POST the iframe calls `play()` on a `<video>` element, and the call resolves. No `playback_error` or other SDK error fired in any run. `PUT /v1/me/player/play` returned 204. Position passed 30 s by `player_state_changed` and reached 36 s before the SDK pause.

**Origin.** Neither the custom scheme nor `127.0.0.1` needed https. Both report `isSecureContext: true`, and the SDK iframe is https anyway. No custom user agent was needed. WKWebView's default UA has no `Safari/` token, and the SDK didn't care.

**Autoplay.** Not needed on macOS. With `with_autoplay(false)` the iframe's `play()` still resolves with no user gesture. Keep it on in the real app anyway, since it costs nothing.

**Hidden window.** A window that is never shown, with the accessory activation policy, plays the same as a visible one. Default background throttling was fine.

**Now Playing.** The webview shows up on its own, owned by `com.apple.WebKit.GPU` (the pid of the webview's GPU process). The title is "Spotify Embedded Player", the iframe's document title, with no artist or album. `navigator.mediaSession.metadata` set from the top frame never reached Now Playing, because the media element lives in the SDK's cross-origin iframe. The first `get` 33 s into playback reported `elapsedTime` near 0, so elapsed time is stale too. The port needs its own answer for metadata.

**Toggle.** `media-control toggle-play-pause` reaches the SDK. The first toggle produced `player_state_changed paused=true` about 30 ms later, the second `paused=false`. The top-frame `mediaSession` action handlers never fired. WebKit drives the iframe's media element directly and the SDK notices.

**Memory while playing** (RSS, hidden run):

| Process                         | RSS        |
| ------------------------------- | ---------- |
| wkwebview-playback (Rust + tao) | 77 MB      |
| com.apple.WebKit.GPU            | 58 MB      |
| com.apple.WebKit.WebContent     | 45 MB      |
| com.apple.WebKit.Networking     | 24-29 MB   |
| Total                           | 206-215 MB |

WebKit's helpers are XPC services with ppid 1, so the spike attributes them by diffing `com.apple.WebKit.*` pids before and after it creates the webview. It would miss nothing and count nothing extra unless another WebKit app started a helper during the run.

**Audible.** Someone at the machine has to confirm this by ear. The spike can only show `play()` resolving, `playbackRate: 1` in Now Playing, and the position advancing.
