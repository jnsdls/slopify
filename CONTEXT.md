# slopify

A macOS menu-bar player for one Premium Spotify account. It plays a Source through the Web Playback SDK inside the app itself, with Spotify desktop closed.

## Language

**Listener**:
The Spotify account signed in to the app. Must hold Premium and sit on the developer app's allowlist.
_Avoid_: user, account, member

**Source**:
The thing playing: a Playlist or Liked Songs. Exactly one Source is current at a time.
_Avoid_: context, collection, library, queue

**Playlist**:
A Spotify playlist the Listener owns, follows, or has pasted a link to.
_Avoid_: list, mix

**Liked Songs**:
The Listener's saved tracks, played as one Source.
_Avoid_: saved tracks, favourites, library, collection

**Pasted Playlist**:
A Playlist reached by pasting its link. It is the Source only until another Source is picked.
_Avoid_: custom playlist, external playlist

**Player**:
The Web Playback SDK instance in the app's renderer. It is the Spotify Connect device audio comes out of.
_Avoid_: device, SDK, engine

**Dropdown**:
The window that opens under the menu bar icon. The whole user interface.
_Avoid_: popover, panel, popup, menu

**Picker**:
The part of the Dropdown that lists Sources. Opens in place from the "Playing from" row.
_Avoid_: source list, browser, library view

**Resume Point**:
The Source, track and position the app returns to at launch, paused.
_Avoid_: last played, bookmark, session

**Sign-in State**:
What the Dropdown shows when the app has no usable Listener: first run, a dead refresh token, or a non-Premium account.
_Avoid_: logged out, auth screen, onboarding

## Parked terms

Kept for the day Radio returns. Not used in v1.

**Radio**:
A station generated from a Seed by a language model, resolved into Spotify tracks.

**Seed**:
The text or track a Radio grows from.

**Batch**:
One set of tracks Radio produces at a time.
