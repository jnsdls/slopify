// Pure helpers for the renderer. No DOM, no bridge, so Vitest runs them under node.

export interface Image {
  url: string;
  width: number | null;
  height: number | null;
}

export function formatTime(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${seconds.toString().padStart(2, '0')}`;
}

export function largestImage(images: readonly Image[]): Image | undefined {
  let best: Image | undefined;
  for (const image of images) {
    if (!best || (image.width ?? 0) > (best.width ?? 0)) best = image;
  }
  return best;
}

/** `spotify:<type>:<id>` to its open.spotify.com page, or null for anything else. */
export function spotifyUrl(uri: string, type: string): string | null {
  const parts = uri.split(':');
  if (parts.length !== 3 || parts[0] !== 'spotify' || parts[1] !== type || !parts[2]) return null;
  return `https://open.spotify.com/${type}/${parts[2]}`;
}

export function trackUrl(uri: string): string | null {
  return spotifyUrl(uri, 'track');
}

export function artistUrl(uri: string): string | null {
  return spotifyUrl(uri, 'artist');
}

/** Counts playback errors and trips once `limit` land within `windowMs`. Stays tripped. */
export class PlaybackErrorGate {
  private readonly times: number[] = [];
  private tripped = false;

  constructor(
    private readonly limit = 3,
    private readonly windowMs = 60_000,
  ) {}

  record(now = Date.now()): boolean {
    if (this.tripped) return true;
    this.times.push(now);
    while (this.times.length && now - (this.times[0] as number) > this.windowMs) this.times.shift();
    if (this.times.length >= this.limit) this.tripped = true;
    return this.tripped;
  }

  get isTripped(): boolean {
    return this.tripped;
  }
}

export interface Scheduled<A extends unknown[]> {
  (...args: A): void;
  /** Runs a pending trailing call now. */
  flush(): void;
  /** Drops a pending call and resets the window. */
  cancel(): void;
}

/** Leading call goes through at once, later calls collapse into one trailing call per window. */
export function throttle<A extends unknown[]>(fn: (...args: A) => void, ms: number): Scheduled<A> {
  let last = -Infinity;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pending: A | null = null;

  const run = (args: A) => {
    last = Date.now();
    pending = null;
    fn(...args);
  };

  const throttled = ((...args: A) => {
    const wait = ms - (Date.now() - last);
    if (wait <= 0 && !timer) {
      run(args);
      return;
    }
    pending = args;
    if (!timer) {
      timer = setTimeout(
        () => {
          timer = null;
          if (pending) run(pending);
        },
        Math.max(wait, 0),
      );
    }
  }) as Scheduled<A>;

  throttled.flush = () => {
    if (timer) clearTimeout(timer);
    timer = null;
    if (pending) run(pending);
  };
  throttled.cancel = () => {
    if (timer) clearTimeout(timer);
    timer = null;
    pending = null;
    last = -Infinity;
  };
  return throttled;
}

/** Calls once, `ms` after the last call. */
export function debounce<A extends unknown[]>(fn: (...args: A) => void, ms: number): Scheduled<A> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pending: A | null = null;

  const debounced = ((...args: A) => {
    pending = args;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      const args = pending;
      pending = null;
      if (args) fn(...args);
    }, ms);
  }) as Scheduled<A>;

  debounced.flush = () => {
    if (timer) clearTimeout(timer);
    timer = null;
    const args = pending;
    pending = null;
    if (args) fn(...args);
  };
  debounced.cancel = () => {
    if (timer) clearTimeout(timer);
    timer = null;
    pending = null;
  };
  return debounced;
}
