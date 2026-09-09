import { describe, expect, it } from 'vitest';
import {
  CLIENT_ID,
  REDIRECT_URI,
  SCOPES,
  buildAuthorizeUrl,
  computeChallenge,
  generatePkce,
} from './pkce.js';

describe('computeChallenge', () => {
  it('matches the RFC 7636 appendix B vector', () => {
    expect(computeChallenge('dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk')).toBe(
      'E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM',
    );
  });
});

describe('generatePkce', () => {
  it('derives a 64-byte base64url verifier and its S256 challenge from the random source', () => {
    const calls: number[] = [];
    const randomBytes = (n: number) => {
      calls.push(n);
      return Buffer.alloc(n, 0xfb);
    };
    const { verifier, challenge, state } = generatePkce(randomBytes);
    expect(calls[0]).toBe(64);
    expect(verifier).toMatch(/^[A-Za-z0-9_-]+$/);
    expect(verifier).toBe(Buffer.alloc(64, 0xfb).toString('base64url'));
    expect(challenge).toBe(computeChallenge(verifier));
    expect(state).toMatch(/^[A-Za-z0-9_-]+$/);
    expect(state.length).toBeGreaterThan(0);
  });

  it('gives a different verifier and state per call', () => {
    const a = generatePkce();
    const b = generatePkce();
    expect(a.verifier).not.toBe(b.verifier);
    expect(a.state).not.toBe(b.state);
  });
});

describe('buildAuthorizeUrl', () => {
  const url = new URL(buildAuthorizeUrl({ challenge: 'CHAL', state: 'STATE' }));

  it('targets the Spotify authorize endpoint', () => {
    expect(url.origin + url.pathname).toBe('https://accounts.spotify.com/authorize');
  });

  it('carries the client id, redirect, response type and PKCE fields', () => {
    expect(url.searchParams.get('client_id')).toBe(CLIENT_ID);
    expect(url.searchParams.get('redirect_uri')).toBe(REDIRECT_URI);
    expect(url.searchParams.get('response_type')).toBe('code');
    expect(url.searchParams.get('code_challenge_method')).toBe('S256');
    expect(url.searchParams.get('code_challenge')).toBe('CHAL');
    expect(url.searchParams.get('state')).toBe('STATE');
  });

  it('carries every scope', () => {
    const scopes = url.searchParams.get('scope')?.split(' ') ?? [];
    expect(SCOPES).toHaveLength(7);
    expect(scopes).toEqual([...SCOPES]);
  });
});
