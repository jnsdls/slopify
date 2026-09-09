import { describe, expect, it, vi } from 'vitest';
import type { Source } from '../../shared/bridge';
import { ApiError, type SpotifyApi } from './api';
import { buildPlayBody, getPlayingElsewhere, startSource, transferHere } from './playback';

const playlist: Source = { kind: 'playlist', id: 'abc', uri: 'spotify:playlist:abc', name: 'P', imageUrl: null, pasted: false };
const liked: Source = { kind: 'liked' };

type Route = (call: { method: string; path: string; body?: unknown }) => { status: number; json: unknown } | ApiError;

function fakeApi(route: Route = () => ({ status: 200, json: null })) {
  const calls: { method: string; path: string; query?: Record<string, string>; body?: unknown }[] = [];
  const request = vi.fn(async (method: string, path: string, opts?: { query?: Record<string, string>; body?: unknown }) => {
    const call = { method, path, query: opts?.query, body: opts?.body };
    calls.push(call);
    const r = route(call);
    if (r instanceof ApiError) throw r;
    return r;
  });
  const log = { info: vi.fn(), warn: vi.fn() };
  return { api: { request } as unknown as SpotifyApi, calls, log };
}

describe('buildPlayBody', () => {
  it('plays a playlist by uri', () => {
    expect(buildPlayBody(playlist, 'me')).toEqual({ context_uri: 'spotify:playlist:abc' });
  });

  it('plays liked songs through the collection uri', () => {
    expect(buildPlayBody(liked, 'me')).toEqual({ context_uri: 'spotify:user:me:collection' });
  });

  it('adds offset and position when resuming at a track', () => {
    expect(buildPlayBody(liked, 'me', { trackUri: 'spotify:track:t', positionMs: 1234 })).toEqual({
      context_uri: 'spotify:user:me:collection',
      offset: { uri: 'spotify:track:t' },
      position_ms: 1234,
    });
  });

  it('ignores a resume point without a track', () => {
    expect(buildPlayBody(playlist, 'me', { trackUri: null, positionMs: 1234 })).toEqual({ context_uri: 'spotify:playlist:abc' });
  });
});

describe('startSource', () => {
  it('plays, then sets shuffle off, then repeat context, all on the device', async () => {
    const { api, calls } = fakeApi();
    await startSource(api, { source: playlist, userId: 'me', deviceId: 'dev' });
    expect(calls).toEqual([
      { method: 'PUT', path: '/me/player/play', query: { device_id: 'dev' }, body: { context_uri: 'spotify:playlist:abc' } },
      { method: 'PUT', path: '/me/player/shuffle', query: { state: 'false', device_id: 'dev' }, body: undefined },
      { method: 'PUT', path: '/me/player/repeat', query: { state: 'context', device_id: 'dev' }, body: undefined },
    ]);
  });

  it('retries without the offset when the play call fails with 4xx', async () => {
    const { api, calls } = fakeApi(({ path, body }) =>
      path === '/me/player/play' && (body as { offset?: unknown }).offset ? new ApiError('PUT', path, 404, null) : { status: 200, json: null },
    );
    await startSource(api, { source: playlist, userId: 'me', deviceId: 'dev', resume: { trackUri: 'spotify:track:t', positionMs: 5 } });
    expect(calls.map((c) => c.path)).toEqual(['/me/player/play', '/me/player/play', '/me/player/shuffle', '/me/player/repeat']);
    expect(calls[0]?.body).toEqual({ context_uri: 'spotify:playlist:abc', offset: { uri: 'spotify:track:t' }, position_ms: 5 });
    expect(calls[1]?.body).toEqual({ context_uri: 'spotify:playlist:abc' });
  });

  it('throws play-failed with the status when the retry also fails', async () => {
    const { api, calls } = fakeApi(({ path }) => (path === '/me/player/play' ? new ApiError('PUT', path, 404, null) : { status: 200, json: null }));
    await expect(
      startSource(api, { source: playlist, userId: 'me', deviceId: 'dev', resume: { trackUri: 'spotify:track:t', positionMs: 5 } }),
    ).rejects.toEqual({ code: 'play-failed', status: 404 });
    expect(calls).toHaveLength(2);
  });

  it('does not retry a 4xx when there was no offset to drop', async () => {
    const { api, calls } = fakeApi(({ path }) => (path === '/me/player/play' ? new ApiError('PUT', path, 403, null) : { status: 200, json: null }));
    await expect(startSource(api, { source: playlist, userId: 'me', deviceId: 'dev' })).rejects.toEqual({ code: 'play-failed', status: 403 });
    expect(calls).toHaveLength(1);
  });

  it('does not retry a 5xx without the offset', async () => {
    const { api, calls } = fakeApi(({ path }) => (path === '/me/player/play' ? new ApiError('PUT', path, 502, null) : { status: 200, json: null }));
    await expect(
      startSource(api, { source: playlist, userId: 'me', deviceId: 'dev', resume: { trackUri: 'spotify:track:t', positionMs: 5 } }),
    ).rejects.toEqual({ code: 'play-failed', status: 502 });
    expect(calls).toHaveLength(1);
  });

  it('logs but does not throw when shuffle or repeat fail', async () => {
    const { api, calls, log } = fakeApi(({ path }) => (path === '/me/player/play' ? { status: 200, json: null } : new ApiError('PUT', path, 500, null)));
    await startSource(api, { source: liked, userId: 'me', deviceId: 'dev' }, log);
    expect(calls.map((c) => c.path)).toEqual(['/me/player/play', '/me/player/shuffle', '/me/player/repeat']);
    expect(log.warn).toHaveBeenCalledTimes(2);
  });
});

describe('transferHere', () => {
  it('moves playback to the device and starts it', async () => {
    const { api, calls } = fakeApi();
    await transferHere(api, 'dev');
    expect(calls).toEqual([{ method: 'PUT', path: '/me/player', query: undefined, body: { device_ids: ['dev'], play: true } }]);
  });
});

describe('getPlayingElsewhere', () => {
  const state = (deviceId: string, item?: unknown) => ({ device: { id: deviceId, name: 'Kitchen' }, item });
  const track = {
    uri: 'spotify:track:t1',
    name: 'Song',
    artists: [{ name: 'A', uri: 'spotify:artist:a' }, { name: 'B', uri: 'spotify:artist:b' }],
    album: { name: 'Album', images: [{ url: 'small.jpg', width: 64 }, { url: 'big.jpg', width: 640 }, { url: 'mid.jpg', width: 300 }] },
  };

  it('returns null on 204', async () => {
    const { api } = fakeApi(() => ({ status: 204, json: null }));
    expect(await getPlayingElsewhere(api, 'own')).toBeNull();
  });

  it('returns null when the active device is our own', async () => {
    const { api } = fakeApi(() => ({ status: 200, json: state('own', track) }));
    expect(await getPlayingElsewhere(api, 'own')).toBeNull();
  });

  it('returns the device and track when another device is active', async () => {
    const { api } = fakeApi(() => ({ status: 200, json: state('other', track) }));
    expect(await getPlayingElsewhere(api, 'own')).toEqual({
      deviceName: 'Kitchen',
      track: {
        uri: 'spotify:track:t1',
        name: 'Song',
        artists: [{ name: 'A', uri: 'spotify:artist:a' }, { name: 'B', uri: 'spotify:artist:b' }],
        album: 'Album',
        imageUrl: 'big.jpg',
      },
    });
  });

  it('returns just the device name when there is no item', async () => {
    const { api } = fakeApi(() => ({ status: 200, json: state('other') }));
    expect(await getPlayingElsewhere(api, null)).toEqual({ deviceName: 'Kitchen' });
  });
});
