# Spotify attribution for the slopify dropdown

Resolves issue #15 (part of #2). Read 2026-09-08 against the Developer Policy, the Developer Terms, the Design Guidelines and the Web Playback SDK docs. Nothing here comes from a blog post or a forum thread. Where a page says nothing, the checklist says so instead of guessing.

Sources:

- Policy: https://developer.spotify.com/policy
- Terms: https://developer.spotify.com/terms
- Design: https://developer.spotify.com/documentation/design
- SDK: https://developer.spotify.com/documentation/web-playback-sdk and https://developer.spotify.com/documentation/web-playback-sdk/reference

## The short answer

Three things are hard requirements and the rest is layout advice. The dropdown must (1) carry the Spotify mark whenever it shows Spotify metadata or is playing audio, (2) link the metadata back to the matching entity on Spotify, and (3) never play audio without artwork and metadata on screen. The decided design already does all three: footer mark, Spotify link beside the title, artwork plus title and artist above the controls.

The smallest compliant set for a 300px dropdown is the Spotify icon at 21px or larger in the footer (white on the dark theme, black on the light theme), one link on the track title to `open.spotify.com/track/<id>` (or the `spotify:track:` URI), uncropped square artwork with 4px corners, and title and artist rendered exactly as the SDK returns them.

## Checklist

Each line is marked required (the page says "must", "never", "only" or "do not") or recommended (the page says "should", "recommend" or "it's OK to"). Source URLs are after each item.

### Logo and icon

- Required. Attribute Spotify content with the Spotify mark. "If you use any Spotify metadata (including artist, album and track names, album artwork, and audio playback) it must always be accompanied by the Spotify brand." The policy says the same with legal force: "If you display any Spotify Content you must clearly attribute the content as being supplied and made available by Spotify, by using the Spotify Marks." https://developer.spotify.com/documentation/design (Attribution), https://developer.spotify.com/policy (II)
- Required. Either the full logo or the icon satisfies attribution in a playing view: "you must always attribute content from Spotify with either the Spotify logo or icon." https://developer.spotify.com/documentation/design (Playing views)
- Recommended. Prefer the full logo; fall back to the icon when there is no room. "Use the icon on its own only if you do not have enough room for the full logo or in cases when the Spotify brand has already been established." The Attribution section is stricter ("In partner integrations, you should always use our full logo") and lists the app-icon case as the one explicit exception. A 300px footer that also holds the user name and Quit qualifies as not enough room, but the safer reading is full logo when it fits. https://developer.spotify.com/documentation/design (Using our logo, Attribution)
- Required. Never the wordmark without the icon. https://developer.spotify.com/documentation/design (Using our logo)
- Required. Minimum size. Full logo 70px wide, icon 21px, in digital. https://developer.spotify.com/documentation/design (Using our logo)
- Required. Clear space equal to half the icon's height on every side, kept free of other UI. https://developer.spotify.com/documentation/design (Using our logo)
- Required. Do not rotate, stretch, recolour outside the palette, fill the lines, use the mark as a letter in a sentence, or place it on a busy or low-contrast area. https://developer.spotify.com/documentation/design (Logo misuse)
- Required. Use the files Spotify ships, not a redrawn mark. Full logo: `/images/guidelines/design/2024-spotify-full-logo.zip`. Icon: `/images/guidelines/design/2024-spotify-logo-icon.zip`. https://developer.spotify.com/documentation/design (Using our logo)

### Colour

- Required. Green mark only on pure black or pure white. "The Spotify green logo should only be used on a black or white background, for any other background you should use a monochrome logo." A macOS vibrancy panel is neither, so the dropdown uses monochrome. https://developer.spotify.com/documentation/design (Using our logo)
- Required. White mark on dark backgrounds, black mark on light backgrounds. https://developer.spotify.com/documentation/design (Using our logo)
- Recommended. No hex value for Spotify Green appears on the design page. If a green mark is ever wanted, take the colour from the shipped asset rather than a guessed hex. https://developer.spotify.com/documentation/design (Using our colors)

### Links back to Spotify

- Required. Metadata must link back. "If you use any Spotify metadata (including artist, album and track names, album artwork and audio playback) it must always link back to the Spotify Service." https://developer.spotify.com/documentation/design (Linking to Spotify)
- Required. The link goes to the matching entity, not the Spotify home page. "Metadata, cover art and Audio Preview Clips must be accompanied by a link back to the applicable album, content or playlist on the Spotify Service." https://developer.spotify.com/policy (II)
- Recommended. One link is enough. The design page does not require a separate link per field. The SDK hands over `current_track.uri`, `artists[].uri` and `context.uri`, so linking title to the track, artist name to the artist, and "Playing from" to the playlist costs nothing and reads as deliberate. Minimum: the track link. https://developer.spotify.com/documentation/web-playback-sdk/reference (Player state), https://developer.spotify.com/documentation/design (Linking to Spotify)
- Recommended. Label text when the link is a button rather than the title itself: "OPEN SPOTIFY", "PLAY ON SPOTIFY" or "LISTEN ON SPOTIFY" when the app is installed, "GET SPOTIFY FREE" when it is not. An icon-only link beside the title is not forbidden by the page; the text strings are given as the options when text is shown. https://developer.spotify.com/documentation/design (Linking to Spotify)
- Recommended. Neither the design page nor the policy says whether to open `spotify:` URIs or `https://open.spotify.com` URLs. On macOS, `open.spotify.com` hands off to the desktop app when installed and falls back to the browser otherwise, which covers both cases with one link. https://developer.spotify.com/documentation/design (Linking to Spotify)

### Artwork

- Required. No cropping. "Don't crop the artwork in any way." Square artwork stays square. https://developer.spotify.com/documentation/design (Using our content)
- Required. No overlays. "Don't overlay images or text on top of the artwork. Don't cover the artwork with playback controls." No play button on the art, no gradient with the title over it, no blur behind the panel taken from the art. https://developer.spotify.com/documentation/design (Using our content)
- Required. No animation or distortion, including blur. https://developer.spotify.com/documentation/design (Using our content)
- Required. No slopify mark on the art. https://developer.spotify.com/documentation/design (Using our content)
- Required. Rounded corners. "Artwork corners must be rounded to create optical blending with nearby UI elements." 4px on small and medium devices, 8px on large. A 300px panel is small. https://developer.spotify.com/documentation/design (Using our content)
- Recommended. Background colour may be extracted from the artwork; the fallback is `#191414`. A solid background derived from the art is allowed, a blurred copy of the art is not. https://developer.spotify.com/documentation/design (Using our content)
- Recommended. Artwork may be dropped entirely when space is limited. https://developer.spotify.com/documentation/design (Using our content)

### Metadata

- Required. Show it as Spotify sends it. "Track, artist, playlist, and album titles must always be presented with the metadata provided by Spotify" and "Don't manipulate any content or metadata." No title-casing, no stripping "(feat. ...)". https://developer.spotify.com/documentation/design (Using our content)
- Required. Legible. https://developer.spotify.com/documentation/design (Using our content)
- Required. Truncation is allowed but the full string must be reachable. "You may truncate metadata if space is limited. The user should always be able to view the entire metadata." A tooltip or a marquee on hover satisfies this. https://developer.spotify.com/documentation/design (Using our content)
- Recommended. Reserve room for 23 characters of track name, 18 of artist name and 25 of playlist or album name before truncating. https://developer.spotify.com/documentation/design (Using our content)
- Required. No playback without artwork and metadata on screen. "there shall be no playback of Spotify Content without showing relevant cover art and metadata in your SDA." The panel closes while audio keeps playing; see the menu bar note below. https://developer.spotify.com/policy (II)
- Recommended. Album name is not in the required set. The design page lists track, artist, playlist and album as things that must be rendered faithfully when shown, not as a set that must all be shown. Title and artist are enough. https://developer.spotify.com/documentation/design (Using our content)

### Playing view, controls, Now Playing

- Required. The dropdown is a playing view and the rules apply to it. "In all playback views where content from Spotify is playing (fullscreen views, widgets, bars, skipped song notifications) make sure to follow these guidelines." https://developer.spotify.com/documentation/design (Playing views)
- Required. Always link to the Spotify app when a client exists on the platform. macOS has one. https://developer.spotify.com/documentation/design (Playing views)
- Recommended. Only play/pause. "It is recommended that no play controls other than play/pause are provided in your app." The reason given is Free-tier restrictions confusing users. Every slopify listener is Premium, so next is defensible, and the SDK's `disallows.skipping_next` should still gate it. https://developer.spotify.com/documentation/design (Playing views), https://developer.spotify.com/documentation/web-playback-sdk/reference
- Recommended. If the progress bar is not seekable, make that visible: "there should be no indication that the user can seek." Premium allows seeking, so this only matters if the bar is display-only by choice. https://developer.spotify.com/documentation/design (Playing views)
- Recommended. If a Like button is ever added, it uses the + icon, writes back to Spotify, and shows "Added to Liked Songs" / "Removed from Liked Songs". Liked content must not be stored by the app. https://developer.spotify.com/documentation/design (Playing views)
- Menu bar icon. No page mentions a menu bar, system tray, status item or mini player. The nearest text is the playing-view list above. Reading it strictly, the collapsed menu bar icon is not a playback view because it shows no content, so the attribution rules do not attach to it. The app's own icon must not resemble the Spotify mark or use "Spotify Green, the circle, and the waves". A plain glyph in the template-image style is fine; a green circle is not. https://developer.spotify.com/documentation/design (Logo and naming restrictions)
- macOS Now Playing. Not mentioned anywhere. The Now Playing widget is Apple's surface; Spotify's rules cover slopify's own views. Passing title, artist and artwork through unmodified keeps it inside the metadata rules anyway.

### App name and branding

- Required. The app name must not include "Spotify", start with "Spot", or sound like it. "slopify" ends in "-ify" and the policy tests "confusing in sound or spelling to Spotify". It does not begin with "Spot" and the first syllable differs, so it passes the letter of the rule. Worth a second opinion before any public release. https://developer.spotify.com/policy (VI), https://developer.spotify.com/documentation/design (Logo and naming restrictions)
- Required. "for Spotify" is acceptable phrasing; implying endorsement is not. https://developer.spotify.com/documentation/design (Logo and naming restrictions)
- Required. Do not pair the Spotify mark with another brand. The footer holds the Spotify mark and the user name; keep any slopify wordmark away from it. https://developer.spotify.com/documentation/design (Logo and naming restrictions)

## Policy rules a menu bar player could trip

- Replicating a core experience. "Do not build products or services that mimic, or replicate or attempt to replace a core user experience of Spotify ... without our prior written permission" and "Your product or service must add independent value or functionality that improves users' interactions with Spotify." A player that plays one playlist and Liked Songs is close to the line. The independent value is the menu bar placement and the one-click source. The spec should say that in one sentence so the value is on record. https://developer.spotify.com/policy (III)
- Caching. Metadata and cover art may be cached temporarily to improve performance; audio may not be cached at all except as Conditional Downloads, which the Web Playback SDK does not offer. Keep artwork in memory or a short-lived disk cache and do not persist a track database. "Do not store Spotify Content indefinitely." https://developer.spotify.com/terms (IV.3.1, IV.3.2)
- Playback without a visible view. The policy forbids playback "without showing relevant cover art and metadata in your SDA". The dropdown is closed most of the time while audio plays. The defensible reading is that the view exists and is one click away, the same as a minimised desktop app, and macOS Now Playing shows the same metadata system-wide. The spec should not add a mode that hides metadata on purpose. https://developer.spotify.com/policy (II)
- Premium only. "Streaming of music sound recordings through the Spotify Platform shall only be made available to subscribers to the Premium Spotify Service." The SDK enforces this, and the allowlist should not admit Free accounts. https://developer.spotify.com/policy (IV), https://developer.spotify.com/documentation/web-playback-sdk
- Non-commercial. "the SDK must not be used in commercial projects without Spotify's prior written approval." No paid tier, no ads, no sponsorships in a Streaming SDA. https://developer.spotify.com/documentation/web-playback-sdk, https://developer.spotify.com/policy (IV)
- Single source to many listeners. "Do not create an application which plays content from a single source to several simultaneous listeners." Each allowlisted user plays their own stream under their own account; do not add a shared-session feature. https://developer.spotify.com/policy (III)
- Mixing. No crossfade or overlap with other audio, including other Spotify audio. https://developer.spotify.com/policy (III)
- Disconnect. Users need an "easily accessible mechanism to disconnect their Spotify account". A Sign out item next to Quit covers it. https://developer.spotify.com/policy (I)
- Privacy policy. Required for any SDA, even one with five users. A page in the repo is enough. https://developer.spotify.com/policy (I)
- Connect device name. The SDK's `name` option "will be visible in other Spotify apps". It goes through the same naming rules: not "Spotify", not starting with "Spot". https://developer.spotify.com/documentation/web-playback-sdk/reference, https://developer.spotify.com/policy (VI)
- Electron. The SDK lists Chrome, Firefox, Safari and Edge and never mentions Electron. Initialization errors are attributed to "the browser not supporting EME protection". Running on Electron with Widevine is a supported-browser question, not a policy question, and the SDK page gives no answer either way. https://developer.spotify.com/documentation/web-playback-sdk

## What this changes in the decided design

Two items push on the layout. First, the green mark is out unless the footer sits on pure black or white; the mark is monochrome, white on dark and black on light. Second, no overlay of any kind on the artwork, so the 300px square cannot carry the title, a play glyph, or a gradient, and the panel background cannot be a blurred copy of it. Everything else in the decided layout is already inside the rules.

One item is a judgement call the spec should state rather than leave implicit: icon versus full logo in the footer. The full logo at 70px fits in 300px next to a user name and Quit, so the safer choice is the full logo unless the footer gets crowded.
