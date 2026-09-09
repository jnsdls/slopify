import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PlaybackErrorGate, artistUrl, debounce, formatTime, largestImage, throttle, trackUrl } from './format';

describe('formatTime', () => {
  it('renders m:ss', () => {
    expect(formatTime(0)).toBe('0:00');
    expect(formatTime(999)).toBe('0:00');
    expect(formatTime(1000)).toBe('0:01');
    expect(formatTime(65_000)).toBe('1:05');
    expect(formatTime(598_000)).toBe('9:58');
    expect(formatTime(3_725_000)).toBe('62:05');
  });

  it('clamps negatives to zero', () => {
    expect(formatTime(-5000)).toBe('0:00');
  });
});

describe('largestImage', () => {
  it('picks the widest', () => {
    const images = [
      { url: 'a', width: 64, height: 64 },
      { url: 'b', width: 640, height: 640 },
      { url: 'c', width: 300, height: 300 },
    ];
    expect(largestImage(images)?.url).toBe('b');
  });

  it('handles empty and null sizes', () => {
    expect(largestImage([])).toBeUndefined();
    expect(largestImage([{ url: 'a', width: null, height: null }])?.url).toBe('a');
  });
});

describe('urls', () => {
  it('maps uris to open.spotify.com', () => {
    expect(trackUrl('spotify:track:4uLU6hMCjMI75M1A2tKUQC')).toBe('https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC');
    expect(artistUrl('spotify:artist:abc')).toBe('https://open.spotify.com/artist/abc');
  });

  it('rejects the wrong kind of uri', () => {
    expect(trackUrl('spotify:artist:abc')).toBeNull();
    expect(artistUrl('spotify:track:abc')).toBeNull();
    expect(trackUrl('')).toBeNull();
    expect(trackUrl('spotify:track:')).toBeNull();
  });
});

describe('PlaybackErrorGate', () => {
  it('trips on the third error within a minute', () => {
    const gate = new PlaybackErrorGate();
    expect(gate.record(0)).toBe(false);
    expect(gate.record(10_000)).toBe(false);
    expect(gate.record(59_000)).toBe(true);
    expect(gate.isTripped).toBe(true);
  });

  it('forgets errors older than a minute', () => {
    const gate = new PlaybackErrorGate();
    gate.record(0);
    gate.record(10_000);
    expect(gate.record(61_000)).toBe(false);
    expect(gate.record(62_000)).toBe(true);
  });

  it('stays tripped', () => {
    const gate = new PlaybackErrorGate(1, 1000);
    expect(gate.record(0)).toBe(true);
    expect(gate.record(1_000_000)).toBe(true);
  });
});

describe('throttle', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('runs the first call at once and collapses the rest into a trailing call', () => {
    const fn = vi.fn();
    const t = throttle(fn, 5000);
    t(1);
    t(2);
    t(3);
    expect(fn.mock.calls).toEqual([[1]]);
    vi.advanceTimersByTime(4999);
    expect(fn).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1);
    expect(fn.mock.calls).toEqual([[1], [3]]);
  });

  it('lets a call through once the window has passed', () => {
    const fn = vi.fn();
    const t = throttle(fn, 5000);
    t(1);
    vi.advanceTimersByTime(5000);
    t(2);
    expect(fn.mock.calls).toEqual([[1], [2]]);
  });

  it('flush runs the pending call, cancel drops it and reopens the window', () => {
    const fn = vi.fn();
    const t = throttle(fn, 5000);
    t(1);
    t(2);
    t.flush();
    expect(fn.mock.calls).toEqual([[1], [2]]);
    t(3);
    t.cancel();
    vi.advanceTimersByTime(10_000);
    expect(fn).toHaveBeenCalledTimes(2);
    t(4);
    expect(fn.mock.calls[2]).toEqual([4]);
  });
});

describe('debounce', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('calls once with the last arguments after the delay', () => {
    const fn = vi.fn();
    const d = debounce(fn, 300);
    d(0.1);
    vi.advanceTimersByTime(200);
    d(0.2);
    vi.advanceTimersByTime(299);
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(fn.mock.calls).toEqual([[0.2]]);
  });

  it('flush and cancel', () => {
    const fn = vi.fn();
    const d = debounce(fn, 300);
    d(1);
    d.flush();
    expect(fn.mock.calls).toEqual([[1]]);
    d(2);
    d.cancel();
    vi.advanceTimersByTime(1000);
    expect(fn).toHaveBeenCalledTimes(1);
  });
});
