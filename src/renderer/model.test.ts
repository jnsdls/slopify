import { describe, expect, it } from 'vitest';
import type { PlayerState } from './model';
import { pasteErrorLine, pickerRows, sameSource, signInView, sourceForContext, statusOf } from './model';
import type { Source } from '../shared/bridge';

const track = { id: 't', uri: 'spotify:track:t', name: 'Song', artists: [], album: 'Album', imageUrl: null };

function state(patch: Partial<PlayerState> = {}): PlayerState {
  return {
    connected: true,
    track: null,
    paused: true,
    positionMs: 0,
    durationMs: 0,
    volume: 0.5,
    source: null,
    elsewhere: null,
    message: null,
    ...patch,
  };
}

const playlist = (id: string, pasted = false): Source => ({
  kind: 'playlist',
  id,
  uri: `spotify:playlist:${id}`,
  name: id,
  imageUrl: null,
  pasted,
});

describe('statusOf', () => {
  it('orders reconnecting over elsewhere over empty', () => {
    expect(statusOf(state({ connected: false, elsewhere: 'Phone', track }))).toBe('reconnecting');
    expect(statusOf(state({ elsewhere: 'Phone', track }))).toBe('elsewhere');
    expect(statusOf(state())).toBe('empty');
    expect(statusOf(state({ track, paused: false }))).toBe('playing');
    expect(statusOf(state({ track, paused: true }))).toBe('paused');
  });
});

describe('signInView', () => {
  it('maps every reason to its line', () => {
    expect(signInView({ kind: 'signed-out', reason: 'first-run' }, null)?.line).toBe('Sign in to start playing.');
    expect(signInView({ kind: 'signed-out', reason: 'expired' }, null)?.line).toBe(
      'Spotify signed you out. Sign in again to keep going.',
    );
    expect(signInView({ kind: 'signed-out', reason: 'not-premium' }, null)?.line).toBe('slopify needs a Premium account.');
    expect(signInView({ kind: 'signed-out', reason: 'error', detail: 'Port 8888 is in use' }, null)?.line).toBe(
      'Port 8888 is in use',
    );
  });

  it('disables the button while signing in', () => {
    expect(signInView({ kind: 'signing-in' }, null)?.waiting).toBe(true);
  });

  it('is null when signed in unless the renderer hit a fatal error', () => {
    const signedIn = { kind: 'signed-in', displayName: 'j', userId: 'u' } as const;
    expect(signInView(signedIn, null)).toBeNull();
    expect(signInView(signedIn, 'Authentication failed')).toEqual({ line: 'Authentication failed', waiting: false });
  });
});

describe('pasteErrorLine', () => {
  it('has one line for bad links and one for playlists Spotify hides', () => {
    expect(pasteErrorLine('bad-link')).toBe("That's not a playlist link");
    expect(pasteErrorLine('not-found')).toBe("Spotify won't share that playlist with this app");
    expect(pasteErrorLine('forbidden')).toBe("Spotify won't share that playlist with this app");
  });
});

describe('sameSource', () => {
  it('compares by kind and id', () => {
    expect(sameSource({ kind: 'liked' }, { kind: 'liked' })).toBe(true);
    expect(sameSource(playlist('a'), playlist('a', true))).toBe(true);
    expect(sameSource(playlist('a'), playlist('b'))).toBe(false);
    expect(sameSource(playlist('a'), { kind: 'liked' })).toBe(false);
    expect(sameSource(null, null)).toBe(true);
    expect(sameSource(null, playlist('a'))).toBe(false);
  });
});

describe('sourceForContext', () => {
  const sources: Source[] = [{ kind: 'liked' }, playlist('a')];

  it('finds playlists by uri and Liked Songs by the collection uri', () => {
    expect(sourceForContext('spotify:playlist:a', sources, 'me')).toEqual(playlist('a'));
    expect(sourceForContext('spotify:user:me:collection', sources, 'me')).toEqual({ kind: 'liked' });
  });

  it('returns null for unknown or missing contexts', () => {
    expect(sourceForContext('spotify:playlist:zzz', sources, 'me')).toBeNull();
    expect(sourceForContext(null, sources, 'me')).toBeNull();
    expect(sourceForContext('spotify:user:other:collection', sources, 'me')).toBeNull();
  });
});

describe('pickerRows', () => {
  it('appends the current Pasted Playlist without persisting it in the list', () => {
    const sources: Source[] = [{ kind: 'liked' }, playlist('a')];
    const pasted = playlist('p', true);
    expect(pickerRows(sources, pasted)).toEqual([...sources, pasted]);
    expect(pickerRows(sources, playlist('a'))).toEqual(sources);
    expect(sources).toHaveLength(2);
  });

  it('shows at least Liked Songs before the list has loaded', () => {
    expect(pickerRows([], null)).toEqual([{ kind: 'liked' }]);
  });
});
