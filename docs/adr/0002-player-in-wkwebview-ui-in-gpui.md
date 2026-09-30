# Player in a hidden WKWebView, UI in GPUI

Supersedes [ADR 0001](0001-in-app-playback-via-castlabs-electron.md).

slopify becomes a Rust app. The Player stays Spotify's Web Playback SDK, but it runs in a hidden WKWebView through `wry` instead of castLabs Electron. macOS supplies FairPlay to every WKWebView, and the SDK uses it: the [spike](https://github.com/jnsdls/slopify/issues/22) got a 200 from Spotify's FairPlay licence server and played past 30 s with no Widevine, no castLabs build and no VMP signature. The castLabs pin, the EVS account and the `sign-pkg` release step all go away. The release script only has to ad-hoc codesign.

The Dropdown is drawn with GPUI, taken from the `zed-industries/zed` repository at a pinned release tag (currently `v1.22.0`, commit `76659a5`). The crates.io release is a year stale, and the `gpui-ce` fork has fewer users than Zed itself. Zed only merges what Zed needs, so bumping the pin is a deliberate step and gets tested like one.

The SDK runs in a cross-origin iframe, so WebKit's automatic Now Playing entry says "Spotify Embedded Player" and nothing more. The app owns Now Playing through MediaPlayer.framework instead ([#29](https://github.com/jnsdls/slopify/issues/29)).

## Considered options

librespot plays Spotify from Rust with no web view. Rejected: it's unofficial, Spotify has been refusing its audio keys since mid-2026, and Spotify has asked the maintainers not to get around its protections.

A SwiftUI or AppKit UI around the same WKWebView would be less work than GPUI and just as small. Rejected because the owner chose GPUI.
