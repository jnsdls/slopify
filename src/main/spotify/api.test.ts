import { describe, expect, it, vi } from 'vitest';
import { ApiError, SpotifyApi } from './api';

type Reply = { status: number; body?: string; headers?: Record<string, string> };

function fakeFetch(replies: Reply[]) {
  const calls: { url: string; init: RequestInit }[] = [];
  const fetch = vi.fn(async (url: string | URL | Request, init?: RequestInit) => {
    calls.push({ url: String(url), init: init ?? {} });
    const r = replies.shift();
    if (!r) throw new Error('fake fetch: no reply left');
    return { status: r.status, headers: new Headers(r.headers), text: async () => r.body ?? '' } as Response;
  });
  return { fetch: fetch as unknown as typeof globalThis.fetch, calls };
}

function make(replies: Reply[], extra: { token?: string; refreshed?: string } = {}) {
  const { fetch, calls } = fakeFetch(replies);
  const sleep = vi.fn(async () => {});
  const log = { info: vi.fn(), warn: vi.fn() };
  const getAccessToken = vi.fn(async () => extra.token ?? 'tok1');
  const forceRefresh = vi.fn(async () => extra.refreshed ?? 'tok2');
  const api = new SpotifyApi({ getAccessToken, forceRefresh, fetch, sleep, log });
  return { api, calls, sleep, log, getAccessToken, forceRefresh };
}

function auth(call: { init: RequestInit }) {
  return (call.init.headers as Record<string, string>)['Authorization'];
}

describe('SpotifyApi.request', () => {
  it('builds the url, sends the bearer token and parses json', async () => {
    const { api, calls } = make([{ status: 200, body: '{"a":1}' }]);
    const r = await api.request<{ a: number }>('GET', '/me/playlists', { query: { limit: '50' } });
    expect(r).toEqual({ status: 200, json: { a: 1 } });
    expect(calls[0]?.url).toBe('https://api.spotify.com/v1/me/playlists?limit=50');
    expect(calls[0]?.init.method).toBe('GET');
    expect(auth(calls[0]!)).toBe('Bearer tok1');
  });

  it('serialises the body as json', async () => {
    const { api, calls } = make([{ status: 200 }]);
    await api.request('PUT', '/me/player', { body: { device_ids: ['d'] } });
    expect(calls[0]?.init.body).toBe('{"device_ids":["d"]}');
    expect((calls[0]?.init.headers as Record<string, string>)['Content-Type']).toBe('application/json');
  });

  it('accepts an absolute url so paging can follow next', async () => {
    const { api, calls } = make([{ status: 200, body: '{}' }]);
    await api.request('GET', 'https://api.spotify.com/v1/me/playlists?offset=50&limit=50');
    expect(calls[0]?.url).toBe('https://api.spotify.com/v1/me/playlists?offset=50&limit=50');
  });

  it('gives json null for a 200 with an empty body', async () => {
    const { api } = make([{ status: 200 }]);
    expect(await api.request('PUT', '/me/player/shuffle')).toEqual({ status: 200, json: null });
  });

  it('gives json null for a 200 whose body is not JSON', async () => {
    const { api } = make([{ status: 200, body: 'Hl7vflUgbZ' }]);
    expect(await api.request('PUT', '/me/player/repeat')).toEqual({ status: 200, json: null });
  });

  it('resolves any 2xx', async () => {
    const { api } = make([{ status: 204 }]);
    expect(await api.request('PUT', '/me/player/play')).toEqual({ status: 204, json: null });
  });

  it('on 401 refreshes once and retries with the new token', async () => {
    const { api, calls, forceRefresh } = make([{ status: 401 }, { status: 200, body: '{"ok":true}' }]);
    const r = await api.request('GET', '/me');
    expect(r.json).toEqual({ ok: true });
    expect(forceRefresh).toHaveBeenCalledTimes(1);
    expect(auth(calls[0]!)).toBe('Bearer tok1');
    expect(auth(calls[1]!)).toBe('Bearer tok2');
  });

  it('on a second 401 throws ApiError', async () => {
    const { api, forceRefresh } = make([{ status: 401 }, { status: 401 }]);
    await expect(api.request('GET', '/me')).rejects.toBeInstanceOf(ApiError);
    expect(forceRefresh).toHaveBeenCalledTimes(1);
  });

  it('on 429 waits Retry-After seconds and retries once', async () => {
    const { api, sleep, calls } = make([
      { status: 429, headers: { 'Retry-After': '3' } },
      { status: 200, body: '{}' },
    ]);
    await api.request('GET', '/me');
    expect(sleep).toHaveBeenCalledWith(3000);
    expect(calls).toHaveLength(2);
  });

  it('on 5xx waits 1 s, retries once, then throws', async () => {
    const { api, sleep, calls } = make([{ status: 502 }, { status: 503, body: '{"error":"x"}' }]);
    const err = await api.request('PUT', '/me/player/play').catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    const e = err as ApiError;
    expect(e.status).toBe(503);
    expect(e.body).toEqual({ error: 'x' });
    expect(e.method).toBe('PUT');
    expect(e.path).toBe('/me/player/play');
    expect(sleep).toHaveBeenCalledTimes(1);
    expect(sleep).toHaveBeenCalledWith(1000);
    expect(calls).toHaveLength(2);
  });

  it('throws ApiError on a 404 without retrying', async () => {
    const { api, calls } = make([{ status: 404 }]);
    await expect(api.request('GET', '/playlists/x')).rejects.toMatchObject({ status: 404 });
    expect(calls).toHaveLength(1);
  });

  it('logs every non-2xx as method path status', async () => {
    const { api, log } = make([{ status: 401 }, { status: 404 }]);
    await api.request('GET', '/me').catch(() => {});
    expect(log.warn).toHaveBeenCalledWith('GET /me 401');
    expect(log.warn).toHaveBeenCalledWith('GET /me 404');
  });
});
