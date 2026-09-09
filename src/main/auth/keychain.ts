import { execFile } from 'node:child_process';

export const KEYCHAIN_SERVICE = 'slopify-spotify-refresh-token';

// The CLI, not safeStorage or keytar: the app is ad-hoc signed, so only items
// trusting /usr/bin/security survive a rebuild without a prompt (docs/spec/v1.md, Auth).
const SECURITY_BIN = '/usr/bin/security';
const ITEM_NOT_FOUND = 44;

export interface Keychain {
  read(): Promise<string | null>;
  write(userId: string, token: string): Promise<void>;
  delete(): Promise<void>;
}

export interface SecurityResult {
  code: number;
  stdout: string;
  stderr: string;
}

export type SecurityRunner = (args: string[], stdin?: string) => Promise<SecurityResult>;

export function runSecurity(args: string[], stdin?: string): Promise<SecurityResult> {
  return new Promise((resolve, reject) => {
    const child = execFile(SECURITY_BIN, args, (err, stdout, stderr) => {
      if (!err) return resolve({ code: 0, stdout, stderr });
      if (typeof err.code === 'number') return resolve({ code: err.code, stdout, stderr });
      reject(err);
    });
    child.stdin?.end(stdin ?? '');
  });
}

export function createKeychain(run: SecurityRunner = runSecurity): Keychain {
  const fail = (op: string, r: SecurityResult) =>
    new Error(`security ${op} exited ${r.code}: ${r.stderr.trim()}`);

  return {
    async read() {
      const r = await run(['find-generic-password', '-s', KEYCHAIN_SERVICE, '-w']);
      if (r.code === ITEM_NOT_FOUND) return null;
      if (r.code !== 0) throw fail('find-generic-password', r);
      return r.stdout.replace(/\r?\n$/, '');
    },

    async write(userId, token) {
      const r = await run(
        ['add-generic-password', '-a', userId, '-s', KEYCHAIN_SERVICE, '-U', '-T', SECURITY_BIN, '-w'],
        token,
      );
      if (r.code !== 0) throw fail('add-generic-password', r);
    },

    async delete() {
      const r = await run(['delete-generic-password', '-s', KEYCHAIN_SERVICE]);
      if (r.code !== 0 && r.code !== ITEM_NOT_FOUND) throw fail('delete-generic-password', r);
    },
  };
}
