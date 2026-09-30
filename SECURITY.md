# Security

slopify stores one secret on your machine, the Spotify refresh token, in the login Keychain. It talks to `accounts.spotify.com`, `api.spotify.com`, and Spotify's SDK and licence hosts, and to nothing else. The hidden web view that runs Spotify's SDK talks to Rust only through the messages in `app/src/player.html`.

If you find a way to read that token, reach another origin from the web view, or otherwise get the app to do something the [spec](docs/spec/v1.md) says it must not, report it privately through GitHub's "Report a vulnerability" button on the Security tab rather than in a public issue.

There is no bug bounty.
