# Radio generation: Claude prompt shape and Spotify search resolution

Resolves issue #7. Checked against Anthropic and Spotify docs on 2026-09-08.

## Short answer

One Claude Sonnet 5 call per Batch with structured output (`output_config.format`, JSON schema), the played list sent back as plain "artist - title" lines, and no prompt caching in v1 because a Batch costs about two cents and the cacheable part sits at the caching minimum. Resolve each pair with `GET /search?type=track&q=track:<title> artist:<artist>&limit=10` and verify the artist name app-side. `GET /me/top/artists` and `GET /me/top/tracks` are on Spotify's "endpoints still available" list for Development Mode. Start a Radio with `PUT /me/player/play` and a `uris` array; top up with `POST /me/player/queue`, one URI per call, in sequence. The next button then walks the queue.

## Which Spotify endpoints survive

The February 2026 changelog lists what Development Mode keeps. "Get User's Top Items (`GET /me/top/{type}`)" is under Personalisation; "Search for Item" and "Get Track" are under Metadata; all fifteen player endpoints are under Player, including Start/Resume Playback, Add Item to Queue, Skip to Next and Get User's Queue. Source: https://developer.spotify.com/documentation/web-api/references/changes/february-2026#endpoints-still-available

Neither the November 2024 blog post nor the February 2026 changelog names `/me/top` among removals. The 2024 post removed Related Artists, Recommendations, Audio Features, Audio Analysis, featured and category playlists, preview URLs and editorial playlists for "new apps that are registered on or after today's date". Source: https://developer.spotify.com/blog/2024-11-27-changes-to-the-web-api

Top items take `type` of `artists` or `tracks`, `time_range` (`short_term`, `medium_term`, `long_term`, default medium), `limit` 1 to 50, `offset`, and need the `user-top-read` scope. Source: https://developer.spotify.com/documentation/web-api/reference/get-users-top-artists-and-tracks

The February 2026 rules also require the app owner to hold Premium and cap each Client ID at five users; the July 2026 changelog raised the Client ID limit to 25 per developer and made quota count per developer account. Sources: https://developer.spotify.com/blog/2026-02-06-update-on-developer-access-and-platform-security and https://developer.spotify.com/documentation/web-api/references/changes/july-2026

## Search resolution

The `q` parameter takes field filters: "The available filters are `album`, `artist`, `track`, `year`, `upc`, `tag:hipster`, `tag:new`, `isrc`, and `genre`." `track` applies to tracks only; `artist` applies to albums, artists and tracks. The docs' own example is `q=remaster track:Doxy artist:Miles Davis`. `limit` is "0 - 10", default 5, after the February 2026 change cut the maximum from 50. If a user token is present the user's country overrides `market`. Source: https://developer.spotify.com/documentation/web-api/reference/search

So the query is `track:<title> artist:<artist>` with `type=track`. Nothing in the docs says the filters are exact matches, so treat results as candidates: normalise both sides (case, diacritics, "feat." suffixes, bracketed remaster tags) and take the first result whose `artists[].name` matches the pair. Ask for `limit=10` and pick, rather than trusting position zero. Drop pairs with no match, as the map already decided. Each pair costs one search call against a rolling 30 second rate window, so a Batch that resolves poorly spends 15 calls on fewer than 15 tracks. Source: https://developer.spotify.com/documentation/web-api/concepts/rate-limits

Because search drops tracks, ask Claude for 20 candidates in priority order and stop at 15 resolved. That keeps a Batch near 15 without a second Claude call. A schema of "exactly 15" cannot be enforced anyway: structured outputs support `minItems` only at 0 or 1 and no `maxItems`. Source: https://platform.claude.com/docs/en/build-with-claude/structured-outputs

## Feeding a Batch into playback

`PUT /me/player/play` takes either `context_uri` or a `uris` array of track URIs, plus optional `offset` and `position_ms`. It needs `user-modify-playback-state` and Premium. Nothing on the page documents a cap on `uris` or what happens to an existing queue. Source: https://developer.spotify.com/documentation/web-api/reference/start-a-users-playback

Calling play with a new `uris` list mid-playback restarts playback with that list. To "append" you would resend the whole Radio (played plus new) with `offset` at the current track and `position_ms` at the current position, which is a restart dressed up as a continue, and the map's charting notes say the list breaks past about 180 URIs. Source: https://github.com/jnsdls/slopify/issues/2

`POST /me/player/queue` takes a single `uri` parameter and an optional `device_id`, same scope and Premium rule. Source: https://developer.spotify.com/documentation/web-api/reference/add-to-queue

`POST /me/player/next` "Skips to next track in the user's queue." Source: https://developer.spotify.com/documentation/web-api/reference/skip-users-playback-to-next-track

`GET /me/player/queue` returns `currently_playing` and a `queue` array of the items after it. Source: https://developer.spotify.com/documentation/web-api/reference/get-queue

Both play and queue carry the warning "The order of execution is not guaranteed when you use this API with other Player API endpoints." So a top-up is 15 sequential queue calls, each awaited, not 15 in parallel. Sources: the play and queue pages above.

Recommendation: first Batch through play with `uris`; every later Batch through queue, one call per resolved track, in order. Next then advances into the queue, and Get User's Queue is the check that the top-up landed. What the docs do not say is how queued items interleave with the tail of the original `uris` list before that tail runs out. That is a ten-minute spike with a real device, and the "top up at 3 remaining" rule already leaves slack for it.

## Claude call

Model `claude-sonnet-5`: $2 per million input tokens, $10 per million output, 1M context, adaptive thinking, reliable knowledge through January 2026. Sources: https://platform.claude.com/docs/en/about-claude/models/overview and https://platform.claude.com/docs/en/about-claude/pricing

Structured outputs: put a JSON schema under `output_config.format` with `type: "json_schema"`; every object needs `additionalProperties: false`; `minimum`, `maxLength` and similar constraints are unsupported. In TypeScript, `client.messages.parse` with `zodOutputFormat(schema)` returns `parsed_output` typed from the Zod schema. Changing the schema invalidates the prompt cache for that thread. Source: https://platform.claude.com/docs/en/build-with-claude/structured-outputs

Prompt shape:

System (stable across Batches), three sentences. You name real, released tracks as artist and title pairs for a personal radio. Stay close to the Seed's genre, era and energy; use the Listener's top artists and tracks as taste, not as a list to replay; never repeat anything in the played list. Give titles as they appear on Spotify, without remaster or version suffixes, in priority order.

User message, in this order: the Seed (artist, title, album, year); the taste sketch (top 50 artists by name, top 50 tracks as "artist - title", from `medium_term`); the played list as "artist - title" lines, append-only in play order; the ask ("20 candidates").

Batch schema:

```json
{
  "type": "object",
  "properties": {
    "candidates": {
      "type": "array",
      "minItems": 1,
      "items": {
        "type": "object",
        "properties": {
          "artist": { "type": "string" },
          "title": { "type": "string" }
        },
        "required": ["artist", "title"],
        "additionalProperties": false
      }
    }
  },
  "required": ["candidates"],
  "additionalProperties": false
}
```

Set `output_config.effort` to `low`. Naming 20 songs is recall, not reasoning, and thinking tokens bill as output.

## Cost per Batch

Token estimates use the current tokenizer, which produces roughly 30 percent more tokens than the pre-4.7 one. Source: https://platform.claude.com/docs/en/about-claude/pricing

- System prompt: about 200 tokens.
- Seed plus taste sketch (100 names): about 1,000 tokens.
- Played list at 300 tracks, 12 tokens a line: about 3,600 tokens.
- Output: 20 pairs in JSON, about 400 tokens, plus low-effort thinking, call it 600.

Input 4,800 tokens at $2 per million is $0.0096. Output 600 at $10 per million is $0.006. About $0.016 per Batch with a 300-track played list, under a cent while the Radio is young. A Batch is needed every 12 tracks, so a full day of listening costs a few dozen cents.

## Prompt caching: skip it for v1

Sonnet 5 needs a 1,024-token cacheable prefix; shorter prefixes silently write nothing. Cache reads are $0.20 per million, 5-minute writes $2.50, 1-hour writes $4. Source: https://platform.claude.com/docs/en/build-with-claude/prompt-caching

The system prompt plus taste sketch sits right at that minimum, and caching it saves about $0.002 per Batch. Worse, Batches are roughly 40 minutes apart (12 tracks at 3 to 4 minutes), so the 5-minute cache is cold every time and the 1-hour write costs double the plain input price. The only prefix that is both large and stable is the played list, and it is append-only, so it would cache well if the TTL fitted, which it does not. Keep the played list append-only anyway so caching can be switched on later without a rewrite.

## Open points for the spec

- Spike: queue interleaving with the tail of the initial `uris` list, and the 180-URI figure.
- Taste sketch size (50 and 50 is a guess) and whether to refresh it per Radio or per session.
- Whether a Seed the model does not know should get a "not sure" field in the schema. Costs nothing to add.
