import { mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { StateFile } from './state-file';
import type { ResumePoint } from '../shared/bridge';

const resume: ResumePoint = {
  source: { kind: 'playlist', id: 'abc', uri: 'spotify:playlist:abc', name: 'Mix', imageUrl: null, pasted: false },
  trackUri: 'spotify:track:xyz',
  positionMs: 83210,
};

let dir: string;
let file: string;

beforeEach(async () => {
  dir = await mkdtemp(path.join(tmpdir(), 'slopify-state-'));
  file = path.join(dir, 'state.json');
});

afterEach(async () => {
  await rm(dir, { recursive: true, force: true });
});

describe('StateFile', () => {
  it('treats a missing file as first run with defaults', async () => {
    const s = new StateFile(file);
    await s.load();
    expect(s.firstRun).toBe(true);
    expect(s.getResumePoint()).toBeNull();
    expect(s.getVolume()).toBe(0.5);
  });

  it('round-trips the Resume Point and volume', async () => {
    const a = new StateFile(file, { debounceMs: 5 });
    await a.load();
    a.saveResumePoint(resume);
    a.saveVolume(0.6);
    await a.flush();

    const b = new StateFile(file);
    await b.load();
    expect(b.firstRun).toBe(false);
    expect(b.getResumePoint()).toEqual(resume);
    expect(b.getVolume()).toBe(0.6);
  });

  it('clears the Resume Point', async () => {
    const a = new StateFile(file, { debounceMs: 5 });
    await a.load();
    a.saveResumePoint(resume);
    a.clearResumePoint();
    await a.flush();

    const b = new StateFile(file);
    await b.load();
    expect(b.getResumePoint()).toBeNull();
  });

  it('writes atomically: the target exists and no temp file is left behind', async () => {
    const s = new StateFile(file, { debounceMs: 5 });
    await s.load();
    s.saveVolume(0.3);
    s.saveResumePoint(resume);
    await s.flush();

    const entries = await readdir(dir);
    expect(entries).toEqual(['state.json']);
    const parsed = JSON.parse(await readFile(file, 'utf8'));
    expect(parsed).toEqual({ version: 1, resume, volume: 0.3 });
  });

  it('flushes a pending write before the debounce elapses', async () => {
    const s = new StateFile(file, { debounceMs: 10_000 });
    await s.load();
    s.saveVolume(0.9);
    await s.flush();
    expect(JSON.parse(await readFile(file, 'utf8')).volume).toBe(0.9);
  });

  it('starts fresh on an unknown version, not as first run', async () => {
    await writeFile(file, JSON.stringify({ version: 2, resume, volume: 0.1 }));
    const s = new StateFile(file);
    await s.load();
    expect(s.firstRun).toBe(false);
    expect(s.getResumePoint()).toBeNull();
    expect(s.getVolume()).toBe(0.5);
  });

  it('starts fresh on an unparsable file, not as first run', async () => {
    await writeFile(file, '{not json');
    const s = new StateFile(file);
    await s.load();
    expect(s.firstRun).toBe(false);
    expect(s.getResumePoint()).toBeNull();
    expect(s.getVolume()).toBe(0.5);
  });

  it('flush is a no-op when nothing changed', async () => {
    const s = new StateFile(file);
    await s.load();
    await s.flush();
    expect(await readdir(dir)).toEqual([]);
  });
});
