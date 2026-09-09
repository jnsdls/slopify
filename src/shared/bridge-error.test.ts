import { describe, expect, it } from 'vitest';
import { isBridgeError, parseBridgeError, serializeBridgeError } from './bridge-error';

describe('bridge-error', () => {
  it('round-trips through the Error message ipcMain.handle preserves', () => {
    const thrown = serializeBridgeError({ code: 'play-failed', status: 404, message: 'gone' });
    const wrapped = new Error(`Error invoking remote method 'playback:start': Error: ${thrown.message}`);
    expect(parseBridgeError(wrapped)).toEqual({ code: 'play-failed', status: 404, message: 'gone' });
  });

  it('returns null for ordinary errors', () => {
    expect(parseBridgeError(new Error('boom'))).toBeNull();
    expect(parseBridgeError(new Error('{"code":"nope"}'))).toBeNull();
    expect(parseBridgeError(undefined)).toBeNull();
  });

  it('recognises Error subclasses that carry a known code', () => {
    const err = Object.assign(new Error('bad'), { code: 'bad-link' });
    expect(isBridgeError(err)).toBe(true);
    expect(isBridgeError({ code: 'ENOENT' })).toBe(false);
  });
});
