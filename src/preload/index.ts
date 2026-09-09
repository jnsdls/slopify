import { contextBridge, ipcRenderer, type IpcRendererEvent } from 'electron';
import { ipc, type AuthState, type PlayingElsewhere, type ResumePoint, type SlopifyBridge, type Source } from '../shared/bridge';
import { parseBridgeError } from '../shared/bridge-error';

async function invoke<T>(channel: string, ...args: unknown[]): Promise<T> {
  try {
    return (await ipcRenderer.invoke(channel, ...args)) as T;
  } catch (err) {
    throw parseBridgeError(err) ?? err;
  }
}

function subscribe<T>(channel: string, cb: (value: T) => void): () => void {
  const listener = (_event: IpcRendererEvent, value: T) => cb(value);
  ipcRenderer.on(channel, listener);
  return () => ipcRenderer.removeListener(channel, listener);
}

const bridge: SlopifyBridge = {
  getAuthState: () => invoke<AuthState>(ipc.getAuthState),
  onAuthState: (cb) => subscribe<AuthState>(ipc.authState, cb),
  signIn: () => invoke<void>(ipc.signIn),
  getAccessToken: () => invoke<string>(ipc.getAccessToken),
  reportNotPremium: () => ipcRenderer.send(ipc.reportNotPremium),

  listSources: () => invoke<Source[]>(ipc.listSources),
  resolvePastedLink: (text) => invoke<Source>(ipc.resolvePastedLink, text),

  startSource: (source, resume) => invoke<void>(ipc.startSource, source, resume),
  transferHere: () => invoke<void>(ipc.transferHere),
  getPlayingElsewhere: () => invoke<PlayingElsewhere | null>(ipc.getPlayingElsewhere),

  getResumePoint: () => invoke<ResumePoint | null>(ipc.getResumePoint),
  saveResumePoint: (p) => ipcRenderer.send(ipc.saveResumePoint, p),
  clearResumePoint: () => ipcRenderer.send(ipc.clearResumePoint),
  getVolume: () => invoke<number>(ipc.getVolume),
  saveVolume: (v) => ipcRenderer.send(ipc.saveVolume, v),

  reportDevice: (deviceId) => ipcRenderer.send(ipc.reportDevice, deviceId),

  openExternal: (url) => ipcRenderer.send(ipc.openExternal, url),
  quit: () => ipcRenderer.send(ipc.quit),
  hideWindow: () => ipcRenderer.send(ipc.hideWindow),
  onWindowShown: (cb) => subscribe<boolean>(ipc.windowShown, cb),
  setContentHeight: (px) => ipcRenderer.send(ipc.resize, px),
};

contextBridge.exposeInMainWorld('slopify', bridge);
