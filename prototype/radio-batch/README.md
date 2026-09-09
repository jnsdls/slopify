# Prototype: does Radio produce lists you'd actually play?

Throwaway. Answers [issue #8](https://github.com/jnsdls/slopify/issues/8). Not part of the app.

One command per run, any number of Seeds:

```sh
pnpm install
pnpm batch "Radiohead - Weird Fishes" "Burial - Archangel" --rounds 2
```

Each run opens a browser tab for the Spotify login (one click if you're already signed in), then per Seed:
fetch your top 50 artists and tracks, ask `claude-sonnet-5` for 20 candidates, resolve each through
Spotify search, print HIT or MISS, stop at 15 hits. `--rounds N` asks for N Batches in a row, feeding
the earlier ones back as the played list, to check repeat avoidance.

Every run also writes `runs/<timestamp>-<seed>.md` so the verdict can be recorded from the file.

Needs the Anthropic key in Keychain: `security add-generic-password -a "$USER" -s anthropic-api-key -U -T /usr/bin/security -w`.
Nothing is persisted between runs; the Spotify token lives in memory only.
