import { app, components, session } from 'electron';
import path from 'node:path';
import { TokenStore, createKeychain } from './auth/index';
import { openExternal, registerIpc } from './ipc';
import { log } from './log';
import { SourceCatalog, SpotifyApi } from './spotify/index';
import { StateFile } from './state-file';
import { createTray, setAttention } from './tray';
import { createWindow, getWindow, markQuitting, toggleWindow } from './window';
import { ipc } from '../shared/bridge';

const CSP = [
  "default-src 'self'",
  "script-src 'self' https://sdk.scdn.co",
  "connect-src 'self' https://*.spotify.com https://*.scdn.co wss://*.spotify.com",
  "img-src 'self' https://i.scdn.co data:",
  "style-src 'self' 'unsafe-inline'",
  "media-src 'self' https://*.scdn.co https://*.spotify.com blob:",
  'frame-src https://sdk.scdn.co https://*.spotify.com',
].join('; ');

function installNetworkHooks(): void {
  const { webRequest } = session.defaultSession;

  webRequest.onHeadersReceived((details, callback) => {
    if (details.resourceType !== 'mainFrame') return callback({});
    callback({ responseHeaders: { ...details.responseHeaders, 'Content-Security-Policy': [CSP] } });
  });

  // Licence POST status is on the manual checklist; anything non-2xx goes to the log per the spec.
  const filter = { urls: ['*://*.spotify.com/*', '*://*.scdn.co/*', '*://*.spotifycdn.com/*'] };
  webRequest.onCompleted(filter, (d) => {
    if (d.url.includes('widevine-license') || d.statusCode >= 400) log.info('net', d.statusCode, d.method, d.url.slice(0, 160));
  });
  webRequest.onErrorOccurred(filter, (d) => log.warn('neterr', d.error, d.method, d.url.slice(0, 160)));
}

async function main(): Promise<void> {
  app.dock?.hide();
  log.info('slopify starting', { electron: process.versions.electron, chrome: process.versions.chrome });

  const stateFile = new StateFile(path.join(app.getPath('userData'), 'state.json'), { log });
  await stateFile.load();
  if (stateFile.firstRun) app.setLoginItemSettings({ openAtLogin: true });

  const tokenStore = new TokenStore({ keychain: createKeychain(), openExternal, log });
  const api = new SpotifyApi({
    getAccessToken: () => tokenStore.getAccessToken(),
    forceRefresh: () => tokenStore.forceRefresh(),
    log,
  });
  const catalog = new SourceCatalog(api);

  let flushed = false;
  app.on('before-quit', (event) => {
    markQuitting();
    if (flushed) return;
    event.preventDefault();
    void stateFile.flush().finally(() => {
      flushed = true;
      app.quit();
    });
  });
  app.on('window-all-closed', () => {});

  await components.whenReady();
  log.info('components', components.status());
  installNetworkHooks();

  const tray = createTray({ onToggle: () => toggleWindow(tray) });
  const win = createWindow();
  win.webContents.on('console-message', (event) => {
    if (event.level === 'warning' || event.level === 'error') log.warn('renderer', event.message);
  });

  tokenStore.onState((state) => {
    log.info('auth', state.kind, 'reason' in state ? state.reason : '');
    setAttention(state.kind === 'signed-out' && state.reason === 'expired');
    getWindow()?.webContents.send(ipc.authState, state);
  });
  registerIpc({ tokenStore, api, catalog, stateFile, log });
  await tokenStore.start();
}

app.whenReady().then(main, (err) => {
  log.error('startup failed', err);
  app.exit(1);
});
process.on('unhandledRejection', (err) => log.error('unhandled rejection', err));
