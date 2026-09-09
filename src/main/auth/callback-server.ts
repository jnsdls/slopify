import http from 'node:http';
import type { AddressInfo } from 'node:net';
import { CALLBACK_PORT } from './pkce.js';

export type CallbackErrorCode = 'port-in-use' | 'state-mismatch' | 'denied' | 'timeout';

export class CallbackError extends Error {
  code: CallbackErrorCode;
  constructor(code: CallbackErrorCode, message: string) {
    super(message);
    this.name = 'CallbackError';
    this.code = code;
  }
}

export interface CallbackOptions {
  port?: number;
  host?: string;
  timeoutMs?: number;
}

export interface CallbackServer {
  port: number;
  code: Promise<string>;
  close(): void;
}

const DEFAULT_TIMEOUT_MS = 5 * 60 * 1000;
const CLOSE_TAB_HTML = '<!doctype html><p>Signed in to slopify. You can close this tab.</p>';

export function startCallbackServer(
  expectedState: string,
  opts: CallbackOptions = {},
): Promise<CallbackServer> {
  const { port = CALLBACK_PORT, host = '127.0.0.1', timeoutMs = DEFAULT_TIMEOUT_MS } = opts;

  let settle: { resolve(code: string): void; reject(err: Error): void };
  const code = new Promise<string>((resolve, reject) => {
    settle = { resolve, reject };
  });
  // close() rejects this promise; nobody may be awaiting it by then.
  code.catch(() => undefined);

  const server = http.createServer((req, res) => {
    const url = new URL(req.url ?? '/', `http://${host}`);
    if (url.pathname !== '/callback') {
      res.writeHead(404).end();
      return;
    }
    res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' }).end(CLOSE_TAB_HTML);
    finish(parseCallback(url, expectedState));
  });

  const timer = setTimeout(() => {
    finish({ error: new CallbackError('timeout', 'No callback arrived in time') });
  }, timeoutMs);

  function finish(outcome: { code: string } | { error: CallbackError }) {
    clearTimeout(timer);
    server.close();
    server.closeAllConnections();
    if ('code' in outcome) settle.resolve(outcome.code);
    else settle.reject(outcome.error);
  }

  return new Promise((resolve, reject) => {
    server.once('error', (err: NodeJS.ErrnoException) => {
      clearTimeout(timer);
      reject(
        err.code === 'EADDRINUSE'
          ? new CallbackError('port-in-use', `Port ${port} is in use`)
          : err,
      );
    });
    server.listen(port, host, () => {
      resolve({
        port: (server.address() as AddressInfo).port,
        code,
        close: () => finish({ error: new CallbackError('timeout', 'Callback server closed') }),
      });
    });
  });
}

function parseCallback(
  url: URL,
  expectedState: string,
): { code: string } | { error: CallbackError } {
  const q = url.searchParams;
  if (q.get('state') !== expectedState) {
    return { error: new CallbackError('state-mismatch', 'Callback state did not match') };
  }
  const denied = q.get('error');
  if (denied) return { error: new CallbackError('denied', `Spotify returned ${denied}`) };
  const code = q.get('code');
  if (!code) return { error: new CallbackError('denied', 'Callback carried no code') };
  return { code };
}
