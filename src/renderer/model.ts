// The Dropdown's view model and the pure rules that derive it. No DOM here.

import type { AuthState, PastedLinkErrorCode, SignedOutReason, Source } from '../shared/bridge';

export interface Artist {
  name: string;
  uri: string;
}

export interface TrackInfo {
  id: string | null;
  uri: string;
  name: string;
  artists: Artist[];
  album: string;
  imageUrl: string | null;
}

export interface PlayerState {
  connected: boolean;
  track: TrackInfo | null;
  paused: boolean;
  positionMs: number;
  durationMs: number;
  /** 0..1, mirrors the SDK volume. */
  volume: number;
  source: Source | null;
  /** Device name while another device holds the session. */
  elsewhere: string | null;
  /** Text shown in place of the times, e.g. a playback error. */
  message: string | null;
}

export type Status = 'empty' | 'playing' | 'paused' | 'elsewhere' | 'reconnecting';

export function statusOf(s: PlayerState): Status {
  if (!s.connected) return 'reconnecting';
  if (s.elsewhere !== null) return 'elsewhere';
  if (!s.track) return 'empty';
  return s.paused ? 'paused' : 'playing';
}

/** Sign-in layout input. `fatal` is renderer-local (see player.ts) and wins over the auth state. */
export interface SignInView {
  line: string;
  waiting: boolean;
}

export function signInLine(reason: SignedOutReason, detail?: string): string {
  switch (reason) {
    case 'first-run':
      return 'Sign in to start playing.';
    case 'expired':
      return 'Spotify signed you out. Sign in again to keep going.';
    case 'not-premium':
      return 'slopify needs a Premium account.';
    case 'error':
      return detail ?? 'Something went wrong.';
  }
}

export function signInView(auth: AuthState, fatal: string | null): SignInView | null {
  if (fatal !== null) return { line: fatal, waiting: false };
  if (auth.kind === 'signing-in') return { line: 'Sign in to start playing.', waiting: true };
  if (auth.kind === 'signed-out') return { line: signInLine(auth.reason, auth.detail), waiting: false };
  return null;
}

export function pasteErrorLine(code: PastedLinkErrorCode | string | undefined): string {
  switch (code) {
    case 'bad-link':
      return "That's not a playlist link";
    case 'not-found':
    case 'forbidden':
      return "Spotify won't share that playlist with this app";
    default:
      return "Couldn't open that playlist";
  }
}

export function sourceName(source: Source | null): string {
  if (!source) return 'Choose a source';
  return source.kind === 'liked' ? 'Liked Songs' : source.name;
}

export function sameSource(a: Source | null, b: Source | null): boolean {
  if (!a || !b) return a === b;
  if (a.kind === 'liked' || b.kind === 'liked') return a.kind === b.kind;
  return a.id === b.id;
}

/** Which known Source a context uri belongs to, so the row follows a takeover from another device. */
export function sourceForContext(uri: string | null, sources: readonly Source[], userId: string | null): Source | null {
  if (!uri) return null;
  if (userId && uri === `spotify:user:${userId}:collection`) return sources.find((s) => s.kind === 'liked') ?? { kind: 'liked' };
  return sources.find((s) => s.kind === 'playlist' && s.uri === uri) ?? null;
}

/** Sources shown in the Picker: the fetched list, plus the current Pasted Playlist which is never in the list. */
export function pickerRows(sources: readonly Source[], current: Source | null): Source[] {
  const rows = sources.length ? [...sources] : [{ kind: 'liked' } as Source];
  if (current && current.kind === 'playlist' && current.pasted && !rows.some((s) => sameSource(s, current))) rows.push(current);
  return rows;
}
