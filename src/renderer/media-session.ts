// navigator.mediaSession is the whole media-key and Now Playing mechanism (spec, "Now Playing and media keys").

import { largestImage } from './format';

export interface MediaHandlers {
  play(): void;
  pause(): void;
  next(): void;
  previous(): void;
}

export function installMediaHandlers(handlers: MediaHandlers): void {
  const session = navigator.mediaSession;
  if (!session) return;
  const actions: [MediaSessionAction, () => void][] = [
    ['play', handlers.play],
    ['pause', handlers.pause],
    ['nexttrack', handlers.next],
    ['previoustrack', handlers.previous],
    ['stop', handlers.pause],
  ];
  for (const [action, handler] of actions) {
    try {
      session.setActionHandler(action, handler);
    } catch (error) {
      console.warn('mediaSession handler not supported', action, error);
    }
  }
}

let lastTrackUri: string | null = null;

export function updateMediaSession(state: Spotify.PlaybackState): void {
  const session = navigator.mediaSession;
  if (!session) return;
  const track = state.track_window.current_track;
  if (track.uri !== lastTrackUri) {
    lastTrackUri = track.uri;
    const image = largestImage(track.album.images);
    session.metadata = new MediaMetadata({
      title: track.name,
      artist: track.artists.map((a) => a.name).join(', '),
      album: track.album.name,
      artwork: image ? [{ src: image.url, sizes: `${image.width ?? 0}x${image.height ?? 0}`, type: 'image/jpeg' }] : [],
    });
  }
  session.playbackState = state.paused ? 'paused' : 'playing';
  try {
    session.setPositionState({
      duration: state.duration / 1000,
      position: Math.min(state.position, state.duration) / 1000,
      playbackRate: 1,
    });
  } catch (error) {
    console.warn('mediaSession.setPositionState failed', error);
  }
}

export function clearMediaSession(): void {
  const session = navigator.mediaSession;
  if (!session) return;
  lastTrackUri = null;
  session.metadata = null;
  session.playbackState = 'none';
}
