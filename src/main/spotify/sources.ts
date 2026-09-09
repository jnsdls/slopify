import type { Source } from '../../shared/bridge';
import { bridgeError } from '../../shared/bridge-error';
import { ApiError, type SpotifyApi } from './api';

const ID = '[0-9A-Za-z]{22}';
const FORMS = [
  new RegExp(`^https?://open\\.spotify\\.com/playlist/(${ID})(?:[/?#].*)?$`),
  new RegExp(`^spotify:playlist:(${ID})(?:[?#].*)?$`),
  new RegExp(`^(${ID})$`),
];

export function parsePastedLink(text: string): string | null {
  const trimmed = text.trim();
  for (const form of FORMS) {
    const id = form.exec(trimmed)?.[1];
    if (id) return id;
  }
  return null;
}

interface PlaylistJson {
  id: string;
  uri: string;
  name: string;
  images: { url: string }[] | null;
}

interface Page {
  items: (PlaylistJson | null)[];
  next: string | null;
}

const firstImage = (p: Pick<PlaylistJson, 'images'>) => p.images?.[0]?.url ?? null;

export class SourceCatalog {
  private cache: Source[] | null = null;
  private inflight: Promise<Source[]> | null = null;

  constructor(private readonly api: SpotifyApi) {}

  /** The last fetched list, for callers that must not wait (mapping a context uri to its Source). */
  get current(): Source[] | null {
    return this.cache;
  }

  /** Always fetches; the Picker renders its own cached copy first and swaps in this result. */
  list(): Promise<Source[]> {
    return this.refresh();
  }

  refresh(): Promise<Source[]> {
    this.inflight ??= this.fetchAll().finally(() => {
      this.inflight = null;
    });
    return this.inflight;
  }

  private async fetchAll(): Promise<Source[]> {
    const sources: Source[] = [{ kind: 'liked' }];
    let page = await this.api.request<Page>('GET', '/me/playlists', { query: { limit: '50' } });
    for (;;) {
      for (const p of page.json?.items ?? []) {
        if (p) sources.push({ kind: 'playlist', id: p.id, uri: p.uri, name: p.name, imageUrl: firstImage(p), pasted: false });
      }
      const next = page.json?.next;
      if (!next) break;
      page = await this.api.request<Page>('GET', next);
    }
    this.cache = sources;
    return sources;
  }
}

export async function resolvePastedLink(api: SpotifyApi, text: string): Promise<Source> {
  const id = parsePastedLink(text);
  if (!id) throw bridgeError('bad-link');
  let res;
  try {
    res = await api.request<PlaylistJson>('GET', `/playlists/${id}`, { query: { fields: 'name,uri,images,owner.id' } });
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) throw bridgeError('not-found');
    if (e instanceof ApiError && e.status === 403) throw bridgeError('forbidden');
    throw e;
  }
  const p = res.json;
  if (!p) throw bridgeError('not-found');
  return { kind: 'playlist', id, uri: p.uri, name: p.name, imageUrl: firstImage(p), pasted: true };
}
