export class ApiError extends Error {
  status: number;
  body: unknown;
  method: string;
  path: string;

  constructor(method: string, path: string, status: number, body: unknown) {
    super(`${method} ${path} ${status}`);
    this.name = 'ApiError';
    this.method = method;
    this.path = path;
    this.status = status;
    this.body = body;
  }
}

export interface SpotifyApiDeps {
  getAccessToken: () => Promise<string>;
  forceRefresh: () => Promise<string>;
  fetch?: typeof fetch;
  log?: { info(...a: unknown[]): void; warn(...a: unknown[]): void };
  sleep?: (ms: number) => Promise<void>;
}

export type Method = 'GET' | 'PUT' | 'POST' | 'DELETE';

export interface RequestOpts {
  query?: Record<string, string>;
  body?: unknown;
}

export interface ApiResponse<T> {
  status: number;
  json: T | null;
}

const BASE = 'https://api.spotify.com/v1';

const defaultSleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

export class SpotifyApi {
  private readonly deps: Required<SpotifyApiDeps>;

  constructor(deps: SpotifyApiDeps) {
    this.deps = {
      fetch: globalThis.fetch,
      log: { info: () => {}, warn: () => {} },
      sleep: defaultSleep,
      ...deps,
    };
  }

  /**
   * `path` is either a `/v1`-relative path or an absolute URL. The absolute
   * form exists so callers can follow the `next` links Spotify returns in
   * paged responses.
   */
  async request<T = unknown>(method: Method, path: string, opts: RequestOpts = {}): Promise<ApiResponse<T>> {
    const url = new URL(path.startsWith('https://') ? path : BASE + path);
    for (const [k, v] of Object.entries(opts.query ?? {})) url.searchParams.set(k, v);

    let token = await this.deps.getAccessToken();
    let refreshed = false;
    let waited = false;

    for (;;) {
      const res = await this.send<T>(method, url, token, opts.body);
      if (res.status >= 200 && res.status < 300) return { status: res.status, json: res.json };

      this.deps.log.warn(`${method} ${path} ${res.status}`);

      if (res.status === 401 && !refreshed) {
        refreshed = true;
        token = await this.deps.forceRefresh();
        continue;
      }
      if (res.status === 429 && !waited) {
        waited = true;
        await this.deps.sleep(retryAfterMs(res.retryAfter));
        continue;
      }
      if (res.status >= 500 && !waited) {
        waited = true;
        await this.deps.sleep(1000);
        continue;
      }
      throw new ApiError(method, path, res.status, res.json);
    }
  }

  private async send<T>(
    method: Method,
    url: URL,
    token: string,
    body: unknown,
  ): Promise<ApiResponse<T> & { retryAfter: string | null }> {
    const headers: Record<string, string> = { Authorization: `Bearer ${token}` };
    const init: RequestInit = { method, headers };
    if (body !== undefined) {
      headers['Content-Type'] = 'application/json';
      init.body = JSON.stringify(body);
    }
    const res = await this.deps.fetch(url, init);
    const text = await res.text();
    return {
      status: res.status,
      json: parseJson<T>(text),
      retryAfter: res.headers.get('Retry-After'),
    };
  }
}

function retryAfterMs(header: string | null): number {
  const seconds = Number(header);
  return Number.isFinite(seconds) && seconds > 0 ? seconds * 1000 : 1000;
}

// Spotify answers some PUTs (shuffle, repeat) with 200 and a bare token where the docs say 204.
function parseJson<T>(text: string): T | null {
  if (!text) return null;
  try {
    return JSON.parse(text) as T;
  } catch {
    return null;
  }
}
