// ipcMain.handle rejections reach the renderer as a bare Error with only `message` intact,
// so a BridgeError travels as JSON in that message and the preload turns it back into an object.
import type { BridgeError, BridgeErrorCode } from './bridge';

const CODES: readonly BridgeErrorCode[] = ['bad-link', 'not-found', 'forbidden', 'no-device', 'play-failed'];

export function isBridgeError(err: unknown): err is BridgeError {
  return (
    typeof err === 'object' &&
    err !== null &&
    typeof (err as { code?: unknown }).code === 'string' &&
    CODES.includes((err as { code: BridgeErrorCode }).code)
  );
}

export function serializeBridgeError(err: BridgeError): Error {
  const plain: BridgeError = { code: err.code };
  if (err.status !== undefined) plain.status = err.status;
  if (err.message !== undefined) plain.message = err.message;
  return new Error(JSON.stringify(plain));
}

/** Returns the BridgeError carried by an ipcRenderer.invoke rejection, or null if it is some other error. */
export function parseBridgeError(err: unknown): BridgeError | null {
  const message = err instanceof Error ? err.message : typeof err === 'string' ? err : '';
  const start = message.indexOf('{');
  if (start < 0) return null;
  try {
    const parsed: unknown = JSON.parse(message.slice(start));
    return isBridgeError(parsed) ? parsed : null;
  } catch {
    return null;
  }
}
