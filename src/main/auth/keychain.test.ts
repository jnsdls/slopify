import { describe, expect, it, vi } from 'vitest';
import { KEYCHAIN_SERVICE, createKeychain, type SecurityRunner, type SecurityResult } from './keychain.js';

function fakeRunner(result: Partial<SecurityResult>) {
  const run = vi.fn<SecurityRunner>(async () => ({ code: 0, stdout: '', stderr: '', ...result }));
  return run;
}

describe('createKeychain', () => {
  it('names the service after the spec', () => {
    expect(KEYCHAIN_SERVICE).toBe('slopify-spotify-refresh-token');
  });

  describe('read', () => {
    it('runs find-generic-password -w and returns the trimmed value', async () => {
      const run = fakeRunner({ stdout: 'tok-123\n' });
      await expect(createKeychain(run).read()).resolves.toBe('tok-123');
      expect(run.mock.calls[0]?.[0]).toEqual(['find-generic-password', '-s', KEYCHAIN_SERVICE, '-w']);
    });

    it('treats an empty stored value as no token', async () => {
      const run = fakeRunner({ stdout: '\n' });
      await expect(createKeychain(run).read()).resolves.toBeNull();
    });

    it('returns null when the item is missing (exit 44)', async () => {
      const run = fakeRunner({ code: 44, stderr: 'The specified item could not be found in the keychain.' });
      await expect(createKeychain(run).read()).resolves.toBeNull();
    });

    it('rethrows other failures', async () => {
      const run = fakeRunner({ code: 36, stderr: 'User interaction is not allowed.' });
      await expect(createKeychain(run).read()).rejects.toThrow(/User interaction/);
    });
  });

  describe('write', () => {
    it('feeds add-generic-password to security -i on stdin with the token quoted', async () => {
      const run = fakeRunner({});
      await createKeychain(run).write('user-1', 'tok-123');
      expect(run).toHaveBeenCalledWith(
        ['-i'],
        `add-generic-password -a 'user-1' -s ${KEYCHAIN_SERVICE} -U -T /usr/bin/security -w 'tok-123'\n`,
      );
    });

    it('never puts the token in argv', async () => {
      const run = fakeRunner({});
      await createKeychain(run).write('user-1', 'tok-123');
      expect(run.mock.calls[0]?.[0].join(' ')).not.toContain('tok-123');
    });

    it('escapes single quotes in the token', async () => {
      const run = fakeRunner({});
      await createKeychain(run).write('user-1', "to'k");
      expect(run.mock.calls[0]?.[1]).toContain("-w 'to'\\''k'");
    });

    it('rejects on failure', async () => {
      const run = fakeRunner({ code: 1, stderr: 'nope' });
      await expect(createKeychain(run).write('user-1', 'tok')).rejects.toThrow(/nope/);
    });
  });

  describe('delete', () => {
    it('runs delete-generic-password', async () => {
      const run = fakeRunner({});
      await createKeychain(run).delete();
      expect(run.mock.calls[0]?.[0]).toEqual(['delete-generic-password', '-s', KEYCHAIN_SERVICE]);
    });

    it('treats a missing item as already deleted', async () => {
      const run = fakeRunner({ code: 44 });
      await expect(createKeychain(run).delete()).resolves.toBeUndefined();
    });
  });
});
