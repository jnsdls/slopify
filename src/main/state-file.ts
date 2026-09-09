import { mkdir, readFile, rename, unlink, writeFile } from 'node:fs/promises';
import path from 'node:path';
import type { ResumePoint } from '../shared/bridge';

interface Persisted {
  version: 1;
  resume: ResumePoint | null;
  volume: number;
}

interface Options {
  debounceMs?: number;
  log?: { warn(...args: unknown[]): void };
}

const DEFAULT_VOLUME = 0.5;
const DEBOUNCE_MS = 250;

const fresh = (): Persisted => ({ version: 1, resume: null, volume: DEFAULT_VOLUME });

function parse(raw: string): Persisted | null {
  let json: unknown;
  try {
    json = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof json !== 'object' || json === null) return null;
  const obj = json as Record<string, unknown>;
  if (obj.version !== 1) return null;
  const volume = typeof obj.volume === 'number' && obj.volume >= 0 && obj.volume <= 1 ? obj.volume : DEFAULT_VOLUME;
  const resume = typeof obj.resume === 'object' && obj.resume !== null ? (obj.resume as ResumePoint) : null;
  return { version: 1, resume, volume };
}

async function writeAtomic(target: string, contents: string): Promise<void> {
  const tmp = `${target}.${process.pid}.${Date.now()}.tmp`;
  await mkdir(path.dirname(target), { recursive: true });
  try {
    await writeFile(tmp, contents, 'utf8');
    await rename(tmp, target);
  } catch (err) {
    await unlink(tmp).catch(() => {});
    throw err;
  }
}

/** `<userData>/state.json`: the Resume Point and volume. Writes are debounced and atomic. */
export class StateFile {
  firstRun = false;

  private data: Persisted = fresh();
  private dirty = false;
  private timer: NodeJS.Timeout | null = null;
  private writing: Promise<void> = Promise.resolve();
  private readonly debounceMs: number;
  private readonly log: Options['log'];

  constructor(
    private readonly path: string,
    options: Options = {},
  ) {
    this.debounceMs = options.debounceMs ?? DEBOUNCE_MS;
    this.log = options.log;
  }

  async load(): Promise<void> {
    let raw: string;
    try {
      raw = await readFile(this.path, 'utf8');
    } catch (err) {
      if ((err as NodeJS.ErrnoException).code === 'ENOENT') this.firstRun = true;
      else this.log?.warn('state file unreadable', err);
      this.data = fresh();
      return;
    }
    const parsed = parse(raw);
    if (!parsed) this.log?.warn('state file unusable, starting fresh');
    this.data = parsed ?? fresh();
  }

  getResumePoint(): ResumePoint | null {
    return this.data.resume;
  }

  saveResumePoint(p: ResumePoint): void {
    this.data.resume = p;
    this.schedule();
  }

  clearResumePoint(): void {
    this.data.resume = null;
    this.schedule();
  }

  getVolume(): number {
    return this.data.volume;
  }

  saveVolume(v: number): void {
    this.data.volume = Math.min(1, Math.max(0, v));
    this.schedule();
  }

  /** Writes anything pending now. Call before quit. */
  async flush(): Promise<void> {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    if (this.dirty) this.write();
    await this.writing;
  }

  private schedule(): void {
    this.dirty = true;
    if (this.timer) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      this.write();
    }, this.debounceMs);
  }

  private write(): void {
    this.dirty = false;
    const contents = JSON.stringify(this.data);
    this.writing = this.writing
      .then(() => writeAtomic(this.path, contents))
      .catch((err) => this.log?.warn('state file write failed', err));
  }
}
