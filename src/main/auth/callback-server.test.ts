import { afterEach, describe, expect, it, vi } from 'vitest';
import { CallbackError, startCallbackServer, waitForCallback } from './callback-server.js';

type Started = Awaited<ReturnType<typeof startCallbackServer>>;
const open: Started[] = [];

async function start(state: string, opts: { timeoutMs?: number } = {}) {
  const s = await startCallbackServer(state, { port: 0, ...opts });
  open.push(s);
  return s;
}

const hit = (port: number, path: string) => fetch(`http://127.0.0.1:${port}${path}`);

afterEach(() => {
  for (const s of open.splice(0)) s.close();
  vi.useRealTimers();
});

describe('startCallbackServer', () => {
  it('resolves with the code when the state matches and tells the browser to close the tab', async () => {
    const s = await start('good');
    const res = await hit(s.port, '/callback?code=abc&state=good');
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toContain('text/html');
    expect(await res.text()).toMatch(/close/i);
    await expect(s.code).resolves.toBe('abc');
  });

  it('rejects state-mismatch on the wrong state', async () => {
    const s = await start('good');
    await hit(s.port, '/callback?code=abc&state=evil');
    await expect(s.code).rejects.toMatchObject({ code: 'state-mismatch' });
    await expect(s.code).rejects.toBeInstanceOf(CallbackError);
  });

  it('rejects denied when Spotify sends error', async () => {
    const s = await start('good');
    await hit(s.port, '/callback?error=access_denied&state=good');
    await expect(s.code).rejects.toMatchObject({ code: 'denied' });
  });

  it('answers 404 on other paths and keeps waiting', async () => {
    const s = await start('good');
    const res = await hit(s.port, '/favicon.ico');
    expect(res.status).toBe(404);
    const ok = await hit(s.port, '/callback?code=later&state=good');
    expect(ok.status).toBe(200);
    await expect(s.code).resolves.toBe('later');
  });

  it('closes after the first callback', async () => {
    const s = await start('good');
    await hit(s.port, '/callback?code=abc&state=good');
    await s.code;
    await expect(hit(s.port, '/callback?code=again&state=good')).rejects.toThrow();
  });

  it('rejects port-in-use when the port is taken', async () => {
    const first = await start('a');
    await expect(startCallbackServer('b', { port: first.port })).rejects.toMatchObject({
      code: 'port-in-use',
    });
  });

  it('rejects timeout when nobody calls back', async () => {
    const s = await start('good', { timeoutMs: 20 });
    await expect(s.code).rejects.toMatchObject({ code: 'timeout' });
  });
});

describe('waitForCallback', () => {
  it('surfaces port-in-use as a CallbackError', async () => {
    const first = await start('a');
    await expect(waitForCallback('b', { port: first.port })).rejects.toMatchObject({
      code: 'port-in-use',
    });
  });
});
