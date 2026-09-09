import { describe, expect, it, vi } from 'vitest';
import { ApiError, type SpotifyApi } from './api';
import { SourceCatalog, parsePastedLink, resolvePastedLink } from './sources';

const ID = '37i9dQZF1DXcBWIGoYBM5M';

describe('parsePastedLink', () => {
  it.each([
    [`https://open.spotify.com/playlist/${ID}`, ID],
    [`https://open.spotify.com/playlist/${ID}/`, ID],
    [`https://open.spotify.com/playlist/${ID}?si=abc123&nd=1`, ID],
    [`  https://open.spotify.com/playlist/${ID}?si=abc123 \n`, ID],
    [`http://open.spotify.com/playlist/${ID}`, ID],
    [`spotify:playlist:${ID}`, ID],
    [`spotify:playlist:${ID}?si=x`, ID],
    [ID, ID],
    [`  ${ID}  `, ID],
  ])('accepts %s', (text, id) => {
    expect(parsePastedLink(text)).toBe(id);
  });

  it.each([
    `https://open.spotify.com/track/${ID}`,
    `https://open.spotify.com/album/${ID}?si=x`,
    `https://open.spotify.com/artist/${ID}`,
    `spotify:track:${ID}`,
    `spotify:album:${ID}`,
    `https://example.com/playlist/${ID}`,
    'https://open.spotify.com/playlist/',
    'not a link at all',
    ID.slice(0, 21),
    `${ID}x`,
    `${ID.slice(0, 21)}-`,
    '',
  ])('rejects %s', (text) => {
    expect(parsePastedLink(text)).toBeNull();
  });
});

type Route = (method: string, path: string) => { status: number; json: unknown } | ApiError;

function fakeApi(route: Route) {
  const calls: { method: string; path: string; query?: Record<string, string> }[] = [];
  const request = vi.fn(async (method: string, path: string, opts?: { query?: Record<string, string> }) => {
    calls.push({ method, path, query: opts?.query });
    const r = route(method, path);
    if (r instanceof ApiError) throw r;
    return r;
  });
  return { api: { request } as unknown as SpotifyApi, calls };
}

const playlist = (n: number, images: { url: string }[] = []) => ({
  id: `id${n}`,
  uri: `spotify:playlist:id${n}`,
  name: `Playlist ${n}`,
  images,
});

describe('SourceCatalog', () => {
  it('lists liked first, then playlists across every page', async () => {
    const { api, calls } = fakeApi((_, path) => {
      if (path === '/me/playlists') {
        return {
          status: 200,
          json: { items: [playlist(1, [{ url: 'a.jpg' }, { url: 'b.jpg' }])], next: 'https://api.spotify.com/v1/me/playlists?offset=50&limit=50' },
        };
      }
      if (path === 'https://api.spotify.com/v1/me/playlists?offset=50&limit=50') {
        return { status: 200, json: { items: [playlist(2)], next: null } };
      }
      throw new Error(`unexpected ${path}`);
    });
    const sources = await new SourceCatalog(api).list();
    expect(sources).toEqual([
      { kind: 'liked' },
      { kind: 'playlist', id: 'id1', uri: 'spotify:playlist:id1', name: 'Playlist 1', imageUrl: 'a.jpg', pasted: false },
      { kind: 'playlist', id: 'id2', uri: 'spotify:playlist:id2', name: 'Playlist 2', imageUrl: null, pasted: false },
    ]);
    expect(calls[0]).toEqual({ method: 'GET', path: '/me/playlists', query: { limit: '50' } });
    expect(calls).toHaveLength(2);
  });

  it('returns the cache on a second list and refreshes in the background', async () => {
    let n = 0;
    const { api, calls } = fakeApi(() => ({ status: 200, json: { items: [playlist(++n)], next: null } }));
    const catalog = new SourceCatalog(api);
    const first = await catalog.list();
    expect(first[1]).toMatchObject({ id: 'id1' });

    const second = await catalog.list();
    expect(second).toBe(first);
    await vi.waitFor(() => expect(calls).toHaveLength(2));
    const third = await catalog.list();
    expect(third[1]).toMatchObject({ id: 'id2' });
  });

  it('refresh always fetches and replaces the cache', async () => {
    let n = 0;
    const { api } = fakeApi(() => ({ status: 200, json: { items: [playlist(++n)], next: null } }));
    const catalog = new SourceCatalog(api);
    await catalog.list();
    const refreshed = await catalog.refresh();
    expect(refreshed[1]).toMatchObject({ id: 'id2' });
  });
});

describe('resolvePastedLink', () => {
  it('rejects unparseable text with bad-link before calling the api', async () => {
    const { api, calls } = fakeApi(() => ({ status: 200, json: {} }));
    await expect(resolvePastedLink(api, 'nope')).rejects.toEqual({ code: 'bad-link' });
    expect(calls).toHaveLength(0);
  });

  it('fetches the playlist and returns a pasted source', async () => {
    const { api, calls } = fakeApi(() => ({
      status: 200,
      json: { name: 'Mix', uri: `spotify:playlist:${ID}`, images: [{ url: 'x.jpg' }], owner: { id: 'someone' } },
    }));
    const src = await resolvePastedLink(api, `https://open.spotify.com/playlist/${ID}?si=1`);
    expect(src).toEqual({ kind: 'playlist', id: ID, uri: `spotify:playlist:${ID}`, name: 'Mix', imageUrl: 'x.jpg', pasted: true });
    expect(calls[0]).toEqual({ method: 'GET', path: `/playlists/${ID}`, query: { fields: 'name,uri,images,owner.id' } });
  });

  it('maps 404 to not-found and 403 to forbidden', async () => {
    const at = (status: number) => fakeApi((m, p) => new ApiError(m, p, status, null)).api;
    await expect(resolvePastedLink(at(404), ID)).rejects.toEqual({ code: 'not-found' });
    await expect(resolvePastedLink(at(403), ID)).rejects.toEqual({ code: 'forbidden' });
  });

  it('rethrows other api errors', async () => {
    const { api } = fakeApi((m, p) => new ApiError(m, p, 500, null));
    await expect(resolvePastedLink(api, ID)).rejects.toBeInstanceOf(ApiError);
  });
});
