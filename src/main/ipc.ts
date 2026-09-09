import { app, ipcMain, shell, type IpcMainEvent, type IpcMainInvokeEvent } from 'electron';
import type { TokenStore } from './auth/index';
import { bridgeError } from '../shared/bridge-error';
import type { SourceCatalog, SpotifyApi } from './spotify/index';
import { getPlayingElsewhere, resolvePastedLink, startSource, transferHere } from './spotify/index';
import type { StateFile } from './state-file';
import { hideWindow, setContentHeight } from './window';
import { ipc, type ResumePoint, type Source } from '../shared/bridge';
import { isBridgeError, serializeBridgeError } from '../shared/bridge-error';

export interface IpcDeps {
  tokenStore: TokenStore;
  api: SpotifyApi;
  catalog: SourceCatalog;
  stateFile: StateFile;
  log: { info(...a: unknown[]): void; warn(...a: unknown[]): void; error(...a: unknown[]): void };
}

export function openExternal(url: string): void {
  if (/^https?:\/\//i.test(url)) void shell.openExternal(url);
}

export function registerIpc({ tokenStore, api, catalog, stateFile, log }: IpcDeps): void {
  let deviceId: string | null = null;

  const requireDevice = (): string => {
    if (deviceId === null) throw bridgeError('no-device');
    return deviceId;
  };

  const handle = (channel: string, fn: (event: IpcMainInvokeEvent, ...args: unknown[]) => unknown) =>
    ipcMain.handle(channel, async (event, ...args) => {
      try {
        return await fn(event, ...args);
      } catch (err) {
        if (isBridgeError(err)) throw serializeBridgeError(err);
        log.error(`${channel} failed`, err);
        throw err;
      }
    });
  const on = (channel: string, fn: (event: IpcMainEvent, ...args: unknown[]) => void) => ipcMain.on(channel, fn);

  handle(ipc.getAuthState, () => tokenStore.getState());
  handle(ipc.signIn, () => tokenStore.signIn());
  handle(ipc.getAccessToken, () => tokenStore.getAccessToken());
  on(ipc.reportNotPremium, () => tokenStore.reportNotPremium());

  handle(ipc.listSources, () => catalog.list());
  handle(ipc.resolvePastedLink, (_e, text) => resolvePastedLink(api, String(text ?? '')));

  handle(ipc.startSource, (_e, source, resume) => {
    const userId = tokenStore.userId;
    if (!userId) throw new Error('startSource before sign-in');
    return startSource(
      api,
      { source: source as Source, userId, deviceId: requireDevice(), resume: resume as ResumePoint | undefined },
      log,
    );
  });
  handle(ipc.transferHere, () => transferHere(api, requireDevice()));
  handle(ipc.getPlayingElsewhere, () => getPlayingElsewhere(api, deviceId));

  handle(ipc.getResumePoint, () => stateFile.getResumePoint());
  on(ipc.saveResumePoint, (_e, p) => stateFile.saveResumePoint(p as ResumePoint));
  on(ipc.clearResumePoint, () => stateFile.clearResumePoint());
  handle(ipc.getVolume, () => stateFile.getVolume());
  on(ipc.saveVolume, (_e, v) => {
    if (typeof v === 'number') stateFile.saveVolume(v);
  });

  on(ipc.reportDevice, (_e, id) => {
    deviceId = typeof id === 'string' ? id : null;
    log.info('player device', deviceId);
  });

  on(ipc.openExternal, (_e, url) => openExternal(String(url ?? '')));
  on(ipc.quit, () => app.quit());
  on(ipc.hideWindow, () => hideWindow());
  on(ipc.resize, (_e, px) => setContentHeight(Number(px)));
}
