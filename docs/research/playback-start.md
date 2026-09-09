# How playback starts for a Playlist and for Liked Songs

Research for [issue #13](https://github.com/jnsdls/slopify/issues/13), part of the [v1 map](https://github.com/jnsdls/slopify/issues/2). Checked against developer.spotify.com on 2026-09-09. Everything below assumes the audio path settled in ADR 0001: the Web Playback SDK runs inside castLabs Electron and is the Spotify Connect device we play on. The app is a Development Mode app, so the February 2026 restrictions apply.

## Short answer

A Playlist starts with one call, `PUT /me/player/play?device_id=<sdk device>` with `context_uri: spotify:playlist:<id>`. Liked Songs has no documented context URI. Two routes exist. The undocumented `spotify:user:<id>:collection` URI works as a context (secondary evidence only, no offset allowed). The documented route pages `GET /me/tracks` into a `uris` list, and that list has an undocumented cap: 850 URIs returns 413, and playback degrades around 180. Keep the list at 100 or fewer. Shuffle and repeat are one `PUT` each with `device_id`. The Source list comes from `GET /me/playlists` at `limit=50`, paged by `next`. A pasted URL needs only the base-62 ID, and `GET /playlists/{id}` returns metadata for any playlist the account can see, contents only for playlists the user owns or collaborates on.

## What Development Mode still allows

The February 2026 changelog lists every player endpoint as retained: Get Playback State, Transfer Playback, Get Available Devices, Start/Resume Playback, Pause, Skip, Seek, Set Repeat Mode, Set Playback Volume, Toggle Shuffle, plus queue and recently played. `GET /me/playlists`, `GET /playlists/{id}`, `GET /playlists/{id}/items` and `GET /me/tracks` are also on the available list. Source: https://developer.spotify.com/documentation/web-api/references/changes/february-2026

The migration guide gives the dates. New Development Mode apps got the restrictions on 2026-02-11, existing apps on 2026-03-09. Our client id was registered after that, so we are on the new rules. Source: https://developer.spotify.com/documentation/web-api/tutorials/february-2026-migration-guide

Two restrictions matter for this ticket.

Playlist contents are gated. "Playlist contents (`items`) are only returned for playlists the user owns or collaborates on. For other playlists, only metadata is returned and the `items` field will be absent from the response." Playback through `context_uri` does not need the contents, so a followed playlist still plays. Source: https://developer.spotify.com/documentation/web-api/tutorials/february-2026-migration-guide

Spotify's own playlists are blocked. Since 2024-11-27, Development Mode apps cannot access "Algorithmic and Spotify-owned editorial playlists". Expect a 404 from `GET /playlists/{id}` on a pasted Discover Weekly or Today's Top Hits URL, and expect the play call to fail on the same URI. Source: https://developer.spotify.com/blog/2024-11-27-changes-to-the-web-api

The March 2026 changelog only restores `external_ids`. Nothing about playback. Source: https://developer.spotify.com/documentation/web-api/references/changes/march-2026

## The SDK device

The SDK's `ready` event hands back `device_id`. `activateElement()` must run inside a user gesture before the first play, because "Some browsers prevent autoplay of media by ensuring that all playback is triggered by synchronous event-paths originating from user interaction such as a click." The `autoplay_failed` event fires when the gesture was missing. `setName()` sets what other Spotify clients see in the Connect picker. Source: https://developer.spotify.com/documentation/web-playback-sdk/reference

Every player endpoint below takes `device_id` as a query parameter: "The id of the device this command is targeting. If not supplied, the user's currently active device is the target." Passing the SDK's id on each call means we never depend on which device Spotify thinks is active, and we never need `PUT /me/player` (transfer). Transfer exists if we want it, body `{"device_ids": ["<id>"], "play": true}`, and "only a single device_id is currently supported". Source: https://developer.spotify.com/documentation/web-api/reference/transfer-a-users-playback

Scopes. The SDK how-to requests `streaming user-read-email user-read-private`. The player endpoints need `user-modify-playback-state`, reading state needs `user-read-playback-state`, Liked Songs needs `user-library-read`, the Source list needs `playlist-read-private` and `playlist-read-collaborative`. Sources: https://developer.spotify.com/documentation/web-playback-sdk/howtos/web-app-player and https://developer.spotify.com/documentation/web-api/concepts/scopes

## Start/resume playback, the one endpoint that matters

`PUT /me/player/play?device_id=<id>`, scope `user-modify-playback-state`, Premium only, 204 on success. Body fields, quoted from the reference:

- `context_uri`: "Spotify URI of the context to play. Valid contexts are albums, artists & playlists."
- `uris`: "A JSON array of the Spotify track URIs to play."
- `offset`: "Indicates from where in the context playback should start. Only available when context_uri corresponds to an album or playlist object." Takes `position` (zero based) or `uri`.
- `position_ms`: integer, where in the track to start.

Send `context_uri` or `uris`, not both. Source: https://developer.spotify.com/documentation/web-api/reference/start-a-users-playback

Resuming after a pause goes through the SDK (`player.resume()` or `togglePlay()`), not this endpoint. The endpoint is for starting a Source or restoring one at launch.

## Playlist

Call sequence, from SDK ready to audio:

1. `activateElement()` inside the click that picked the Source.
2. `PUT /me/player/play?device_id=<sdk>` with body `{"context_uri": "spotify:playlist:<id>"}`. Add `"offset": {"position": n}` and `"position_ms": m` to restore a remembered spot at launch (then pause via the SDK, since the map says launch resumes paused).
3. Nothing else. Skip is `player.nextTrack()`, volume is `player.setVolume()`.

The offset form with a playlist context has a long history of silent failures on the archived GitHub tracker (a 2018 report where offset 13 produced no playback, no error), so the resume path needs a real test, not just a 204 check. Source, secondary: https://github.com/spotify/web-api/issues/901

## Liked Songs

The docs give Liked Songs no URI. The URIs and IDs concept page lists track, album, artist, playlist, user and category forms and nothing for the saved-tracks collection. Source: https://developer.spotify.com/documentation/web-api/concepts/spotify-uris-ids

### Route A, the undocumented collection URI

`spotify:user:<user id>:collection` is accepted by the play endpoint as a `context_uri`. Nothing on developer.spotify.com says so. The evidence is all secondary:

- A 2021-10-28 community post shows the API console rejecting `{"context_uri": "spotify:user:malnen132:collection", "offset": {"position": 5}}` with `400 "Can't have offset for context type: COLLECTION"`. The error names a COLLECTION context type, so the server parsed and recognised the URI; only the offset was refused. Source: https://community.spotify.com/t5/Spotify-for-Developers/Spotify-api-play-random-song-from-user-saved-tracks/td-p/5284971
- A 2021-12-03 community post: "Playing the saved songs works using the context URI of a user with :collection appended. But I cannot set an offset for the playback." Source: https://community.spotify.com/t5/Spotify-for-Developers/Playback-of-saved-tracks-with-offset/td-p/5309028
- BarRaider's Stream Deck plugin docs ship it as the supported workaround for Liked Songs. Source: https://docs.barraider.com/faqs/spotify/play-uri-action/

The user id comes from `GET /me` (`id` field), which the February 2026 changelog keeps. The removed user fields are `country`, `email`, `explicit_content`, `followers`, `product`; `id` stays. Source: https://developer.spotify.com/documentation/web-api/references/changes/february-2026

Consequences if it works: shuffle and repeat apply to the whole library as a native context, the SDK's `nextTrack()` walks it, and there is no cap. Consequences of relying on it: no offset, so a launch-time resume has to accept starting from the top (or from wherever Spotify puts it), and Spotify can remove the behaviour without a changelog entry because it was never documented. The evidence is also from 2021. A spike should confirm it still returns 204 and that the SDK's `player_state_changed` reports a context, on the current API and our client id.

### Route B, page GET /me/tracks into a uris list

Documented and safe from removal, capped by an undocumented payload limit.

`GET /me/tracks?limit=50&offset=n`, scope `user-library-read`. "Default: 20. Minimum: 1. Maximum: 50." Response has `items[].track.uri`, `next`, `total`. Order is most recently saved first, which matches the Liked Songs view in the Spotify client. Source: https://developer.spotify.com/documentation/web-api/reference/get-users-saved-tracks

Then `PUT /me/player/play?device_id=<sdk>` with `{"uris": [...]}`.

The cap. The reference has no limit on `uris`. What is on record:

- 850 URIs returned `413 payload too large`, reported 2020-02-26 on the archived tracker, never answered by Spotify. Source, secondary: https://github.com/spotify/web-api/issues/1483
- Around 180 URIs the request succeeds but playback degrades: "Player shows an empty Queue", "after one of the first few songs one song seems to continue playing", "next song never comes", "rewind does not work", "eventually 'unable to play this song' pops up". Reported 2021-07-02, zero replies. Source, secondary: https://community.spotify.com/t5/Spotify-for-Developers/URIs-Array-seems-to-have-an-undocumented-limit/td-p/5230001

So the hard limit is a body-size limit somewhere between 180 and 850 URIs (each `spotify:track:<22 chars>` is 36 bytes, so 850 URIs is roughly 33 KB of JSON), and the soft limit where the player itself misbehaves sits near 180. Neither number is stable, both are one-off reports. Send at most 100 URIs (two pages of `GET /me/tracks`), and when the list runs low, push more with `POST /me/player/queue?uri=<track>&device_id=<sdk>`, one track per call, the same pattern the Radio research settled on for Batches. Queue endpoint source: https://developer.spotify.com/documentation/web-api/reference/add-to-queue

Resume at launch with a `uris` list: the docs restrict `offset` to album and playlist contexts, so do not rely on it. Instead build the list starting from the remembered track (the `GET /me/tracks` offset of that track), so it is element zero, and pass `position_ms`.

### Recommendation

Spike Route A first, since it is one call and gives real shuffle. If the spike passes, use it, and keep Route B as the fallback the code can switch to if the play call ever returns 400 or 404 for the collection URI. If Route A fails, Route B is the whole implementation.

## Shuffle and repeat

`PUT /me/player/shuffle?state=true|false&device_id=<sdk>`. `state` is required: "true : Shuffle user's playback. false : Do not shuffle user's playback." Scope `user-modify-playback-state`, Premium, 204. Source: https://developer.spotify.com/documentation/web-api/reference/toggle-shuffle-for-users-playback

`PUT /me/player/repeat?state=track|context|off&device_id=<sdk>`. "track will repeat the current track. context will repeat the current context. off will turn repeat off." Same scope, 204. Source: https://developer.spotify.com/documentation/web-api/reference/set-repeat-mode-on-users-playback

Both act on whatever is playing, so call them after the play call, not before. With a `uris` list the "context" for repeat is the list. The SDK has no shuffle or repeat method of its own; `player_state_changed` reports `shuffle` and `repeat_mode` so the UI can reflect the result.

## Source list

`GET /me/playlists?limit=50&offset=n`, scope `playlist-read-private` (add `playlist-read-collaborative` to include collaborative ones). "Default: 20. Minimum: 1. Maximum: 50." and "Maximum offset: 100.000." Returns "playlists owned or followed by the current Spotify user", so followed playlists come for free. Page by following `next` until it is `null`; `total` gives the count up front. Each item has `id`, `name`, `uri`, `images`, `owner`. Source: https://developer.spotify.com/documentation/web-api/reference/get-a-list-of-current-users-playlists

`GET /users/{id}/playlists` was removed in February 2026; only the `/me` form remains. Source: https://developer.spotify.com/documentation/web-api/references/changes/february-2026

## Pasted URL

Accept these shapes and pull the 22 character base-62 ID:

- `https://open.spotify.com/playlist/<id>` with optional `?si=...` and other query noise
- `spotify:playlist:<id>`
- `spotify:user:<user>:playlist:<id>`, the pre-2018 form some old links still carry

The concept page defines the Spotify ID as "The base-62 identifier found at the end of the Spotify URI" and gives `http://open.spotify.com/track/<id>` as the URL form. Source: https://developer.spotify.com/documentation/web-api/concepts/spotify-uris-ids

Then `GET /playlists/<id>`, path param "The Spotify ID of the playlist." The response carries `name`, `uri`, `owner`, `images`, and `items` only "for playlists owned by the current user or playlists the user is a collaborator of". `market`, `fields` and `additional_types` are optional; `fields=name,uri,owner,images` keeps the response small. Source: https://developer.spotify.com/documentation/web-api/reference/get-playlist

That one call validates the paste. 200 means the playlist is playable through `context_uri: spotify:playlist:<id>`; store `name` and `uri`. 404 covers a bad ID, a private playlist we cannot see, and Spotify-owned editorial or algorithmic playlists (blocked for Development Mode since 2024-11-27). Show the same "can't open that playlist" message for all three. Source: https://developer.spotify.com/blog/2024-11-27-changes-to-the-web-api

## Call sequences, one per Source

Playlist (own, followed, or pasted):

1. SDK `ready` gives `device_id`; `activateElement()` on the click.
2. `PUT /me/player/play?device_id` body `{"context_uri": "spotify:playlist:<id>", "offset": {"position": n}, "position_ms": m}` (offset and position only on launch resume).
3. `PUT /me/player/shuffle?state=&device_id` and `PUT /me/player/repeat?state=&device_id` if the user's saved settings differ from what `player_state_changed` reports.
4. SDK methods from there.

Liked Songs, Route A:

1. `GET /me` once, cache `id`.
2. Same as Playlist with `{"context_uri": "spotify:user:<id>:collection"}` and no `offset`.

Liked Songs, Route B:

1. `GET /me/tracks?limit=50&offset=k` twice (or once if the library is small), collect `track.uri`.
2. `PUT /me/player/play?device_id` body `{"uris": [up to 100], "position_ms": m}` with the resume track first.
3. Shuffle and repeat as above.
4. Top up with `POST /me/player/queue?uri=&device_id` as the list drains.

## Open items

- Route A needs a spike on the current API before the spec commits to it.
- Playlist offset on resume needs the same spike; the endpoint has failed silently on offsets before.
- Route B's 100-URI figure is a margin under a 2021 anecdote, not a measured limit. If a spike wants a real number, bisect between 100 and 850 on the play endpoint and log the first 413 and the first playback glitch.
