import type { AuthState } from '../../shared/bridge.js';
import { CallbackError, startCallbackServer as defaultStartCallbackServer } from './callback-server.js';
import type { Keychain } from './keychain.js';
import { CALLBACK_PORT, CLIENT_ID, REDIRECT_URI, buildAuthorizeUrl, generatePkce } from './pkce.js';

export interface AuthLog {
  info(...a: unknown[]): void;
  warn(...a: unknown[]): void;
  error(...a: unknown[]): void;
}

export interface TokenStoreDeps {
  keychain: Keychain;
  fetch?: typeof fetch;
  openExternal: (url: string) => void;
  startCallbackServer?: typeof defaultStartCallbackServer;
  now?: () => number;
  setTimeout?: typeof setTimeout;
  clearTimeout?: typeof clearTimeout;
  log?: AuthLog;
}

const TOKEN_URL = 'https://accounts.spotify.com/api/token';
const ME_URL = 'https://api.spotify.com/v1/me';
const REFRESH_LEAD_MS = 5 * 60_000;
const BACKOFF_MS = [2_000, 4_000, 8_000];
const BACKOFF_STEADY_MS = 30_000;

type Timer = ReturnType<typeof setTimeout>;
type Listener = (s: AuthState) => void;

interface TokenResponse {
  access_token: string;
  expires_in: number;
  refresh_token?: string;
}

class TokenEndpointError extends Error {
  constructor(
    readonly status: number,
    readonly code: string | null,
    message: string,
  ) {
    super(message);
    this.name = 'TokenEndpointError';
  }
  get invalidGrant() {
    return this.status === 400 && this.code === 'invalid_grant';
  }
}

const noopLog: AuthLog = { info() {}, warn() {}, error() {} };

export class TokenStore {
  private readonly deps: TokenStoreDeps;
  private readonly log: AuthLog;
  private state: AuthState = { kind: 'signed-out', reason: 'first-run' };
  private readonly listeners = new Set<Listener>();
  private refreshToken: string | null = null;
  private accessToken: string | null = null;
  private expiresAt = 0;
  private profile: { userId: string; displayName: string } | null = null;
  private timer: Timer | null = null;
  private failures = 0;
  private inflight: Promise<string> | null = null;
  private flow: Promise<void> | null = null;

  constructor(deps: TokenStoreDeps) {
    this.deps = deps;
    this.log = deps.log ?? noopLog;
  }

  get userId(): string | null {
    return this.profile?.userId ?? null;
  }

  getState(): AuthState {
    return this.state;
  }

  onState(cb: Listener): () => void {
    this.listeners.add(cb);
    return () => this.listeners.delete(cb);
  }

  async start(): Promise<void> {
    const stored = await this.deps.keychain.read();
    if (!stored) {
      this.publish({ kind: 'signed-out', reason: 'first-run' });
      return;
    }
    this.refreshToken = stored;
    this.publish({ kind: 'signing-in' });
    await this.refresh().catch(() => undefined);
  }

  signIn(): Promise<void> {
    if (!this.flow) {
      this.flow = this.signInOnce().finally(() => {
        this.flow = null;
      });
    }
    return this.flow;
  }

  async getAccessToken(): Promise<string> {
    if (this.state.kind === 'signed-out') throw new Error('Signed out');
    if (this.accessToken && this.msLeft() > REFRESH_LEAD_MS) return this.accessToken;
    try {
      return await this.refresh();
    } catch (err) {
      if (this.accessToken && this.msLeft() > 0) return this.accessToken;
      throw err;
    }
  }

  forceRefresh(): Promise<string> {
    return this.refresh();
  }

  reportNotPremium(): void {
    this.publish({ kind: 'signed-out', reason: 'not-premium' });
  }

  private async signInOnce() {
    this.publish({ kind: 'signing-in' });
    try {
      await this.runPkceFlow();
    } catch (err) {
      this.log.error('sign-in failed', err);
      this.publish({ kind: 'signed-out', reason: 'error', detail: describeSignInError(err) });
    }
  }

  private async runPkceFlow() {
    const { verifier, challenge, state } = generatePkce();
    const startCallbackServer = this.deps.startCallbackServer ?? defaultStartCallbackServer;
    const server = await startCallbackServer(state);
    let code: string;
    try {
      this.deps.openExternal(buildAuthorizeUrl({ challenge, state }));
      code = await server.code;
    } finally {
      server.close();
    }
    const tokens = await this.postToken({
      grant_type: 'authorization_code',
      code,
      redirect_uri: REDIRECT_URI,
      client_id: CLIENT_ID,
      code_verifier: verifier,
    });
    await this.adopt(tokens);
  }

  private refresh(): Promise<string> {
    if (!this.inflight) {
      this.inflight = this.refreshOnce().finally(() => {
        this.inflight = null;
      });
    }
    return this.inflight;
  }

  private async refreshOnce(): Promise<string> {
    if (!this.refreshToken) throw new Error('No refresh token');
    this.clearTimer();
    try {
      const tokens = await this.postToken({
        grant_type: 'refresh_token',
        refresh_token: this.refreshToken,
        client_id: CLIENT_ID,
      });
      await this.adopt(tokens);
      this.failures = 0;
      return tokens.access_token;
    } catch (err) {
      if (err instanceof TokenEndpointError && err.invalidGrant) {
        await this.expire();
      } else {
        this.scheduleRetry(err);
      }
      throw err;
    }
  }

  private async adopt(tokens: TokenResponse) {
    this.accessToken = tokens.access_token;
    this.expiresAt = this.now() + tokens.expires_in * 1000;
    const next = tokens.refresh_token;
    const rotated = next !== undefined && next !== this.refreshToken;
    if (rotated) this.refreshToken = next;
    if (!this.profile) this.profile = await this.fetchProfile(tokens.access_token);
    if (rotated) await this.deps.keychain.write(this.profile.userId, next);
    this.scheduleRefresh();
    if (this.state.kind === 'signing-in') {
      this.publish({ kind: 'signed-in', ...this.profile });
    }
  }

  private async expire() {
    this.log.warn('refresh token rejected with invalid_grant, signing out');
    this.refreshToken = null;
    this.accessToken = null;
    this.clearTimer();
    await this.deps.keychain.delete();
    this.publish({ kind: 'signed-out', reason: 'expired' });
  }

  private scheduleRefresh() {
    this.setTimer(Math.max(0, this.msLeft() - REFRESH_LEAD_MS));
  }

  private scheduleRetry(err: unknown) {
    const delay = BACKOFF_MS[this.failures] ?? BACKOFF_STEADY_MS;
    this.failures += 1;
    this.log.warn(`token refresh failed, retrying in ${delay} ms`, err);
    this.setTimer(delay);
  }

  private setTimer(delayMs: number) {
    this.clearTimer();
    const set = this.deps.setTimeout ?? globalThis.setTimeout;
    this.timer = set(() => {
      this.timer = null;
      this.refresh().catch(() => undefined);
    }, delayMs);
  }

  private clearTimer() {
    if (this.timer === null) return;
    (this.deps.clearTimeout ?? globalThis.clearTimeout)(this.timer);
    this.timer = null;
  }

  private async postToken(form: Record<string, string>): Promise<TokenResponse> {
    const res = await this.fetch(TOKEN_URL, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams(form).toString(),
    });
    const body = (await res.json().catch(() => ({}))) as Record<string, unknown>;
    if (!res.ok) {
      this.log.warn(`POST /api/token ${res.status}`, body);
      const code = typeof body.error === 'string' ? body.error : null;
      const detail = typeof body.error_description === 'string' ? body.error_description : code;
      throw new TokenEndpointError(res.status, code, `Token endpoint ${res.status}: ${detail ?? ''}`.trim());
    }
    return body as unknown as TokenResponse;
  }

  private async fetchProfile(accessToken: string) {
    const res = await this.fetch(ME_URL, { headers: { authorization: `Bearer ${accessToken}` } });
    if (!res.ok) {
      this.log.warn(`GET /v1/me ${res.status}`);
      throw new Error(`GET /v1/me ${res.status}`);
    }
    const me = (await res.json()) as { id: string; display_name: string | null };
    return { userId: me.id, displayName: me.display_name ?? me.id };
  }

  private fetch(url: string, init: RequestInit) {
    return (this.deps.fetch ?? globalThis.fetch)(url, init);
  }

  private now() {
    return (this.deps.now ?? Date.now)();
  }

  private msLeft() {
    return this.expiresAt - this.now();
  }

  private publish(next: AuthState) {
    this.state = next;
    for (const cb of this.listeners) cb(next);
  }
}

function describeSignInError(err: unknown): string {
  if (err instanceof CallbackError && err.code === 'port-in-use') return `Port ${CALLBACK_PORT} is in use`;
  return err instanceof Error ? err.message : String(err);
}
