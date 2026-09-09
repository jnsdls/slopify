import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AuthState } from '../../shared/bridge.js';
import { CallbackError } from './callback-server.js';
import type { Keychain } from './keychain.js';
import { CLIENT_ID, REDIRECT_URI } from './pkce.js';
import { TokenStore, type TokenStoreDeps } from './token-store.js';

const MIN = 60_000;
const TOKEN_URL = 'https://accounts.spotify.com/api/token';
const ME_URL = 'https://api.spotify.com/v1/me';

type Call = { url: string; method: string; body: URLSearchParams | null; headers: Record<string, string> };
type Reply = { status: number; body?: unknown } | Error;

function json(status: number, body: unknown) {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function fakeFetch(replies: { token: Reply[]; me?: Reply[] }) {
  const calls: Call[] = [];
  const fetch = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input);
    const headers = Object.fromEntries(new Headers(init?.headers).entries());
    const body = typeof init?.body === 'string' ? new URLSearchParams(init.body) : null;
    calls.push({ url, method: init?.method ?? 'GET', body, headers });
    const queue = url === TOKEN_URL ? replies.token : replies.me ?? [];
    const reply = queue.length > 1 ? queue.shift() : queue[0];
    if (reply === undefined) throw new Error(`no reply for ${url}`);
    if (reply instanceof Error) throw reply;
    return json(reply.status, reply.body);
  }) as unknown as typeof globalThis.fetch;
  return { fetch, calls };
}

function fakeKeychain(stored: string | null): Keychain & { stored: string | null } {
  const kc = {
    stored,
    read: vi.fn(async () => kc.stored),
    write: vi.fn(async (_user: string, token: string) => {
      kc.stored = token;
    }),
    delete: vi.fn(async () => {
      kc.stored = null;
    }),
  };
  return kc;
}

const tokens = (over: Record<string, unknown> = {}) => ({
  status: 200,
  body: { access_token: 'at-1', token_type: 'Bearer', expires_in: 3600, ...over },
});
const me = { status: 200, body: { id: 'user-1', display_name: 'Jonas' } };

function build(opts: {
  stored?: string | null;
  token?: Reply[];
  me?: Reply[];
  deps?: Partial<TokenStoreDeps>;
} = {}) {
  const keychain = fakeKeychain(opts.stored === undefined ? 'rt-0' : opts.stored);
  const { fetch, calls } = fakeFetch({ token: opts.token ?? [tokens()], me: opts.me ?? [me] });
  const states: AuthState[] = [];
  const openExternal = vi.fn();
  const store = new TokenStore({ keychain, fetch, openExternal, ...opts.deps });
  store.onState((s) => states.push(s));
  return { store, keychain, fetch, calls, states, openExternal };
}

const tokenCalls = (calls: Call[]) => calls.filter((c) => c.url === TOKEN_URL);

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-09-09T00:00:00Z'));
});
afterEach(() => vi.useRealTimers());

describe('start', () => {
  it('publishes signed-out/first-run when the Keychain is empty', async () => {
    const { store, states, fetch } = build({ stored: null });
    await store.start();
    expect(store.getState()).toEqual({ kind: 'signed-out', reason: 'first-run' });
    expect(states).toEqual([{ kind: 'signed-out', reason: 'first-run' }]);
    expect(fetch).not.toHaveBeenCalled();
  });

  it('refreshes with the stored token, loads the profile and publishes signed-in', async () => {
    const { store, calls } = build();
    await store.start();
    const [refresh, profile] = calls;
    expect(refresh?.method).toBe('POST');
    expect(refresh?.headers['content-type']).toBe('application/x-www-form-urlencoded');
    expect(Object.fromEntries(refresh!.body!)).toEqual({
      grant_type: 'refresh_token',
      refresh_token: 'rt-0',
      client_id: CLIENT_ID,
    });
    expect(profile?.url).toBe(ME_URL);
    expect(profile?.headers['authorization']).toBe('Bearer at-1');
    expect(store.getState()).toEqual({ kind: 'signed-in', displayName: 'Jonas', userId: 'user-1' });
    expect(store.userId).toBe('user-1');
  });
});

describe('scheduled refresh', () => {
  it('runs five minutes before expiry', async () => {
    const { store, calls } = build({ token: [tokens(), tokens({ access_token: 'at-2' })] });
    await store.start();
    expect(tokenCalls(calls)).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(55 * MIN - 1);
    expect(tokenCalls(calls)).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(tokenCalls(calls)).toHaveLength(2);
    await expect(store.getAccessToken()).resolves.toBe('at-2');
  });

  it('rotates the refresh token when the response carries one', async () => {
    const { store, keychain, calls } = build({
      token: [tokens({ refresh_token: 'rt-1' }), tokens()],
    });
    await store.start();
    expect(keychain.write).toHaveBeenCalledWith('user-1', 'rt-1');
    await vi.advanceTimersByTimeAsync(55 * MIN);
    expect(tokenCalls(calls)[1]?.body?.get('refresh_token')).toBe('rt-1');
  });

  it('does not touch the Keychain when the refresh token is unchanged', async () => {
    const { store, keychain } = build();
    await store.start();
    expect(keychain.write).not.toHaveBeenCalled();
  });

  it('deletes the Keychain item and publishes expired on invalid_grant', async () => {
    const { store, keychain, states } = build({
      token: [tokens(), { status: 400, body: { error: 'invalid_grant', error_description: 'Refresh token revoked' } }],
    });
    await store.start();
    await vi.advanceTimersByTimeAsync(55 * MIN);
    expect(keychain.delete).toHaveBeenCalledOnce();
    expect(states.at(-1)).toEqual({ kind: 'signed-out', reason: 'expired' });
    await expect(store.getAccessToken()).rejects.toThrow();
  });

  it('backs off 2s, 4s, 8s then every 30s on network errors and stays signed-in', async () => {
    const { store, calls, states, keychain } = build({
      token: [tokens(), new Error('ECONNRESET')],
    });
    await store.start();
    await vi.advanceTimersByTimeAsync(55 * MIN);
    expect(tokenCalls(calls)).toHaveLength(2);
    for (const [delay, count] of [
      [2_000, 3],
      [4_000, 4],
      [8_000, 5],
      [30_000, 6],
      [30_000, 7],
    ] as const) {
      await vi.advanceTimersByTimeAsync(delay - 1);
      expect(tokenCalls(calls)).toHaveLength(count - 1);
      await vi.advanceTimersByTimeAsync(1);
      expect(tokenCalls(calls)).toHaveLength(count);
    }
    expect(store.getState().kind).toBe('signed-in');
    expect(states.filter((s) => s.kind === 'signed-out')).toHaveLength(0);
    expect(keychain.delete).not.toHaveBeenCalled();
  });

  it('treats 5xx like a network error', async () => {
    const { store, calls } = build({
      token: [tokens(), { status: 503, body: {} }],
    });
    await store.start();
    await vi.advanceTimersByTimeAsync(55 * MIN);
    await vi.advanceTimersByTimeAsync(2_000);
    expect(tokenCalls(calls)).toHaveLength(3);
    expect(store.getState().kind).toBe('signed-in');
  });

  it('resets the backoff and reschedules once a retry succeeds', async () => {
    const { store, calls } = build({
      token: [tokens(), new Error('down'), tokens({ access_token: 'at-3' })],
    });
    await store.start();
    await vi.advanceTimersByTimeAsync(55 * MIN + 2_000);
    expect(tokenCalls(calls)).toHaveLength(3);
    await expect(store.getAccessToken()).resolves.toBe('at-3');
    await vi.advanceTimersByTimeAsync(55 * MIN);
    expect(tokenCalls(calls)).toHaveLength(4);
  });
});

describe('getAccessToken', () => {
  it('returns the cached token while more than five minutes remain', async () => {
    const { store, calls } = build();
    await store.start();
    vi.setSystemTime(Date.now() + 54 * MIN);
    await expect(store.getAccessToken()).resolves.toBe('at-1');
    expect(tokenCalls(calls)).toHaveLength(1);
  });

  it('refreshes first when five minutes or less remain', async () => {
    const { store, calls } = build({ token: [tokens(), tokens({ access_token: 'at-2' })] });
    await store.start();
    vi.setSystemTime(Date.now() + 56 * MIN);
    await expect(store.getAccessToken()).resolves.toBe('at-2');
    expect(tokenCalls(calls)).toHaveLength(2);
  });

  it('falls back to the unexpired cached token when the refresh fails', async () => {
    const { store } = build({ token: [tokens(), new Error('down')] });
    await store.start();
    vi.setSystemTime(Date.now() + 56 * MIN);
    await expect(store.getAccessToken()).resolves.toBe('at-1');
  });

  it('rejects while signed out', async () => {
    const { store } = build({ stored: null });
    await store.start();
    await expect(store.getAccessToken()).rejects.toThrow(/signed out/i);
  });

  it('shares one in-flight refresh between callers', async () => {
    const { store, calls } = build({ token: [tokens(), tokens({ access_token: 'at-2' })] });
    await store.start();
    vi.setSystemTime(Date.now() + 56 * MIN);
    const both = await Promise.all([store.getAccessToken(), store.getAccessToken()]);
    expect(both).toEqual(['at-2', 'at-2']);
    expect(tokenCalls(calls)).toHaveLength(2);
  });
});

describe('forceRefresh', () => {
  it('refreshes even when the cached token is fresh', async () => {
    const { store, calls } = build({ token: [tokens(), tokens({ access_token: 'at-2' })] });
    await store.start();
    await expect(store.forceRefresh()).resolves.toBe('at-2');
    expect(tokenCalls(calls)).toHaveLength(2);
  });
});

function callbackServer(outcome: string | Error) {
  const code = typeof outcome === 'string' ? Promise.resolve(outcome) : Promise.reject(outcome);
  code.catch(() => undefined);
  return { port: 8888, code, close: vi.fn() };
}

describe('signIn', () => {
  it('opens the browser, exchanges the code, stores the refresh token and publishes signed-in', async () => {
    const startCallbackServer = vi.fn(async () => callbackServer('code-1'));
    const { store, states, openExternal, keychain, calls } = build({
      stored: null,
      token: [tokens({ refresh_token: 'rt-1' })],
      deps: { startCallbackServer },
    });
    await store.start();
    await store.signIn();

    expect(states.map((s) => s.kind)).toEqual(['signed-out', 'signing-in', 'signed-in']);
    const authorize = new URL(openExternal.mock.calls[0]?.[0] as string);
    expect(authorize.origin + authorize.pathname).toBe('https://accounts.spotify.com/authorize');
    const state = authorize.searchParams.get('state');
    expect(startCallbackServer).toHaveBeenCalledWith(state);

    const exchange = tokenCalls(calls)[0];
    expect(Object.fromEntries(exchange!.body!)).toEqual({
      grant_type: 'authorization_code',
      code: 'code-1',
      client_id: CLIENT_ID,
      redirect_uri: REDIRECT_URI,
      code_verifier: expect.stringMatching(/^[A-Za-z0-9_-]{80,}$/),
    });
    expect(keychain.write).toHaveBeenCalledWith('user-1', 'rt-1');
    expect(store.getState()).toEqual({ kind: 'signed-in', displayName: 'Jonas', userId: 'user-1' });
    await expect(store.getAccessToken()).resolves.toBe('at-1');
  });

  it('schedules the refresh after sign-in', async () => {
    const { store, calls } = build({
      stored: null,
      token: [tokens({ refresh_token: 'rt-1' }), tokens({ access_token: 'at-2' })],
      deps: { startCallbackServer: async () => callbackServer('code-1') },
    });
    await store.start();
    await store.signIn();
    await vi.advanceTimersByTimeAsync(55 * MIN);
    expect(tokenCalls(calls)).toHaveLength(2);
    expect(tokenCalls(calls)[1]?.body?.get('refresh_token')).toBe('rt-1');
  });

  it('publishes signed-out/error when the callback state does not match', async () => {
    const { store, states, keychain } = build({
      stored: null,
      deps: {
        startCallbackServer: async () => callbackServer(new CallbackError('state-mismatch', 'Callback state did not match')),
      },
    });
    await store.start();
    await store.signIn();
    expect(states.at(-1)).toEqual({
      kind: 'signed-out',
      reason: 'error',
      detail: 'Callback state did not match',
    });
    expect(keychain.write).not.toHaveBeenCalled();
  });

  it('reports a busy port in the spec wording', async () => {
    const { store, states } = build({
      stored: null,
      deps: {
        startCallbackServer: async () => {
          throw new CallbackError('port-in-use', 'listen EADDRINUSE');
        },
      },
    });
    await store.start();
    await store.signIn();
    expect(states.at(-1)).toEqual({ kind: 'signed-out', reason: 'error', detail: 'Port 8888 is in use' });
  });

  it('publishes signed-out/error with the message when the exchange fails', async () => {
    const { store, states } = build({
      stored: null,
      token: [{ status: 400, body: { error: 'invalid_grant', error_description: 'Invalid authorization code' } }],
      deps: { startCallbackServer: async () => callbackServer('code-1') },
    });
    await store.start();
    await store.signIn();
    expect(states.at(-1)).toMatchObject({ kind: 'signed-out', reason: 'error' });
    expect((states.at(-1) as { detail?: string }).detail).toMatch(/Invalid authorization code/);
  });
});

describe('reportNotPremium', () => {
  it('publishes not-premium and keeps the refresh token', async () => {
    const { store, keychain } = build();
    await store.start();
    store.reportNotPremium();
    expect(store.getState()).toEqual({ kind: 'signed-out', reason: 'not-premium' });
    expect(keychain.delete).not.toHaveBeenCalled();
    expect(keychain.stored).toBe('rt-0');
  });
});

describe('onState', () => {
  it('stops delivering after unsubscribe', async () => {
    const { store } = build({ stored: null });
    const seen: AuthState[] = [];
    const off = store.onState((s) => seen.push(s));
    off();
    await store.start();
    expect(seen).toEqual([]);
  });
});
