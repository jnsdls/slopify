# Contributing

## Reporting a bug

Open an issue with the bug template. Say what you clicked, what happened, and what you expected. Attach the log from `~/Library/Application Support/slopify/logs/slopify.log` if it is relevant; strip anything that looks like a token first.

## Proposing a feature

Read the "Out of scope" list in [docs/spec/v1.md](docs/spec/v1.md) before opening a feature issue. Radio, seek, shuffle, sign-out, Windows and Linux are all deliberately out, and a PR that adds one of them will be closed with a pointer to that list. Anything else, open an issue first so we can agree on the shape before you spend time on it.

## Sending a pull request

1. Fork, branch from `main`, and follow the [quick start](README.md#quick-start) to get the app running.
2. Make the change. Keep the vocabulary from [CONTEXT.md](CONTEXT.md) in names and comments.
3. Run `pnpm check`. CI runs the same command and a PR cannot merge until it passes.
4. If the change touches playback, sign-in, or packaging, run the parts of the manual checklist in [docs/spec/v1.md](docs/spec/v1.md) that apply, on the packaged `.app`, and say in the PR which ones you ran.
5. Open the PR against `main`. Fill in the template.

Small, single-purpose PRs merge fastest. A behaviour change that the spec does not describe needs a spec edit in the same PR.

## Things you cannot change casually

- The castLabs Electron pin. Spotify's licence check was run against that exact build; bumping it means running the check again ([ADR 0001](docs/adr/0001-in-app-playback-via-castlabs-electron.md)).
- The redirect port. Spotify's dashboard refuses a portless loopback URI, so `8888` is fixed.
- The scopes list. It is the minimum the spec needs; adding one is a feature discussion.

## Licence

By contributing you agree that your contribution is licensed under the [MIT licence](LICENSE).
