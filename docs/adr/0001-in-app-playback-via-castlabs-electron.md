# In-app playback through the castLabs Electron fork

slopify plays audio itself, with Spotify desktop closed. Stock Electron has no Widevine CDM, and Spotify's licence server returns 500 to a CDM carrying only the castLabs development signature, so the app runs on castLabs Electron pinned at `v44.1.0+wvcus` with a production VMP signature from EVS. The pin is deliberate: the licence check ran against this exact build, and any bump has to repeat it.

The packaged app is the only build. The release script refreshes the EVS token from the Keychain, runs `sign-pkg` on the Electron framework inside the bundle, then ad-hoc codesigns. VMP before codesign, or the signature breaks.

## Considered options

Remote mode, driving a Spotify Connect device through the Web API, needs no DRM at all. Rejected because it makes Spotify desktop a runtime dependency, adds a device picker to the dropdown, and loses media keys and AirPods controls unless a native addon is written. It was the fallback if the spike failed. The spike passed.
