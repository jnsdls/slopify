import type { BridgeError, Source } from '../../shared/bridge';
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

  /** Cached list when there is one, with a refresh started behind it; otherwise fetches. */
  async list(): Promise<Source[]> {
    if (this.cache) {
      void this.refresh().catch(() => {});
      return this.cache;
    }
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
  if (!id) throw { code: 'bad-link' } satisfies BridgeError;
  let res;
  try {
    res = await api.request<PlaylistJson>('GET', `/playlists/${id}`, { query: { fields: 'name,uri,images,owner.id' } });
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) throw { code: 'not-found' } satisfies BridgeError;
    if (e instanceof ApiError && e.status === 403) throw { code: 'forbidden' } satisfies BridgeError;
    throw e;
  }
  const p = res.json;
  if (!p) throw { code: 'not-found' } satisfies BridgeError;
  return { kind: 'playlist', id, uri: p.uri, name: p.name, imageUrl: firstImage(p), pasted: true };
}
