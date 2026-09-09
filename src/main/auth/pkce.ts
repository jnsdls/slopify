import crypto from 'node:crypto';

// The id of the Spotify developer app this build signs in through. Set MAIN_VITE_SPOTIFY_CLIENT_ID in
// .env.local; electron.vite.config.ts refuses to build without it.
export const CLIENT_ID: string = import.meta.env.MAIN_VITE_SPOTIFY_CLIENT_ID;
export const REDIRECT_URI = 'http://127.0.0.1:8888/callback';
export const CALLBACK_PORT = 8888;
export const SCOPES: readonly string[] = [
  'streaming',
  'user-read-email',
  'user-read-private',
  'user-read-playback-state',
  'user-modify-playback-state',
  'playlist-read-private',
  'playlist-read-collaborative',
];

const AUTHORIZE_ENDPOINT = 'https://accounts.spotify.com/authorize';

export type RandomBytes = (size: number) => Buffer;

export function computeChallenge(verifier: string): string {
  return crypto.createHash('sha256').update(verifier).digest('base64url');
}

export function generatePkce(randomBytes: RandomBytes = crypto.randomBytes): {
  verifier: string;
  challenge: string;
  state: string;
} {
  const verifier = randomBytes(64).toString('base64url');
  return {
    verifier,
    challenge: computeChallenge(verifier),
    state: randomBytes(16).toString('base64url'),
  };
}

export function buildAuthorizeUrl(p: { challenge: string; state: string }): string {
  const params = new URLSearchParams({
    client_id: CLIENT_ID,
    response_type: 'code',
    redirect_uri: REDIRECT_URI,
    scope: SCOPES.join(' '),
    code_challenge_method: 'S256',
    code_challenge: p.challenge,
    state: p.state,
  });
  return `${AUTHORIZE_ENDPOINT}?${params}`;
}
