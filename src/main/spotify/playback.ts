import type { BridgeError, PlayingElsewhere, Source } from '../../shared/bridge';
import { ApiError, type SpotifyApi } from './api';

export interface Resume {
  trackUri: string | null;
  positionMs: number;
}

export interface PlayBody {
  context_uri: string;
  offset?: { uri: string };
  position_ms?: number;
}

type Log = { warn(...a: unknown[]): void };

export function buildPlayBody(source: Source, userId: string, resume?: Resume): PlayBody {
  // The collection URI is undocumented; docs/spec/v1.md "Sources" records the spike that shows it working.
  const body: PlayBody = {
    context_uri: source.kind === 'liked' ? `spotify:user:${userId}:collection` : `spotify:playlist:${source.id}`,
  };
  if (resume?.trackUri) {
    body.offset = { uri: resume.trackUri };
    body.position_ms = resume.positionMs;
  }
  return body;
}

export async function startSource(
  api: SpotifyApi,
  p: { source: Source; userId: string; deviceId: string; resume?: Resume },
  log: Log = console,
): Promise<void> {
  const body = buildPlayBody(p.source, p.userId, p.resume);
  const play = (b: PlayBody) => api.request('PUT', '/me/player/play', { query: { device_id: p.deviceId }, body: b });
  try {
    await play(body);
  } catch (e) {
    if (!(e instanceof ApiError)) throw e;
    if (!body.offset || e.status < 400 || e.status >= 500) throw playFailed(e);
    await play({ context_uri: body.context_uri }).catch((e2: unknown) => {
      throw e2 instanceof ApiError ? playFailed(e2) : e2;
    });
  }

  const set = (path: string, state: string) =>
    api.request('PUT', path, { query: { state, device_id: p.deviceId } }).catch((e: unknown) => log.warn(`${path} failed`, e));
  await set('/me/player/shuffle', 'false');
  await set('/me/player/repeat', 'context');
}

const playFailed = (e: ApiError): BridgeError => ({ code: 'play-failed', status: e.status });

export async function transferHere(api: SpotifyApi, deviceId: string): Promise<void> {
  await api.request('PUT', '/me/player', { body: { device_ids: [deviceId], play: true } });
}


interface PlayerState {
  device: { id: string | null; name: string };
  item?: {
    uri: string;
    name: string;
    artists?: { name: string; uri: string }[];
    album?: { name?: string; images?: { url: string; width?: number | null }[] };
  } | null;
}

export async function getPlayingElsewhere(api: SpotifyApi, ownDeviceId: string | null): Promise<PlayingElsewhere | null> {
  const { json } = await api.request<PlayerState>('GET', '/me/player');
  if (!json?.device || json.device.id === ownDeviceId) return null;
  const out: PlayingElsewhere = { deviceName: json.device.name };
  if (json.item) {
    out.track = {
      uri: json.item.uri,
      name: json.item.name,
      artists: (json.item.artists ?? []).map((a) => ({ name: a.name, uri: a.uri })),
      album: json.item.album?.name ?? '',
      imageUrl: largest(json.item.album?.images ?? []),
    };
  }
  return out;
}

function largest(images: { url: string; width?: number | null }[]): string | null {
  let best: { url: string; width?: number | null } | null = null;
  for (const img of images) if (!best || (img.width ?? 0) > (best.width ?? 0)) best = img;
  return best?.url ?? null;
}
