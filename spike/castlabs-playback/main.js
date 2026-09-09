// Throwaway spike: can castLabs Electron + Web Playback SDK play a Spotify track?
const { app, components, BrowserWindow, shell, session, ipcMain } = require('electron');
const http = require('http');
const crypto = require('crypto');
const fs = require('fs');
const path = require('path');

const CLIENT_ID = 'a768335a56b648d4a6d11d945d029ce4';
const REDIRECT = 'http://127.0.0.1:8888/callback';
const SCOPES = 'streaming user-read-email user-read-private user-modify-playback-state user-read-playback-state';
const TOKEN_FILE = path.join(app.getPath('userData'), 'spike-token.json');
const LOG = path.join(app.getPath('userData'), 'spike.log');

function log(...a) {
  const line = `${new Date().toISOString()} ${a.map(x => typeof x === 'string' ? x : JSON.stringify(x)).join(' ')}`;
  console.log(line);
  fs.appendFileSync(LOG, line + '\n');
}

const b64url = b => b.toString('base64').replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');

async function exchange(body) {
  const r = await fetch('https://accounts.spotify.com/api/token', {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams(body),
  });
  const j = await r.json();
  if (!r.ok) throw new Error(`token endpoint ${r.status}: ${JSON.stringify(j)}`);
  j.expires_at = Date.now() + j.expires_in * 1000;
  fs.writeFileSync(TOKEN_FILE, JSON.stringify(j));
  return j;
}

async function getToken() {
  if (fs.existsSync(TOKEN_FILE)) {
    const t = JSON.parse(fs.readFileSync(TOKEN_FILE, 'utf8'));
    if (t.expires_at > Date.now() + 60_000) return t.access_token;
    if (t.refresh_token) {
      log('refreshing token');
      const n = await exchange({ grant_type: 'refresh_token', refresh_token: t.refresh_token, client_id: CLIENT_ID });
      return n.access_token;
    }
  }
  const verifier = b64url(crypto.randomBytes(48));
  const challenge = b64url(crypto.createHash('sha256').update(verifier).digest());
  const state = b64url(crypto.randomBytes(12));
  const url = 'https://accounts.spotify.com/authorize?' + new URLSearchParams({
    client_id: CLIENT_ID, response_type: 'code', redirect_uri: REDIRECT, scope: SCOPES,
    code_challenge_method: 'S256', code_challenge: challenge, state,
  });
  const code = await new Promise((resolve, reject) => {
    const srv = http.createServer((req, res) => {
      const u = new URL(req.url, 'http://127.0.0.1:8888');
      if (u.pathname !== '/callback') { res.writeHead(404).end(); return; }
      res.end('<p>slopify spike: you can close this tab.</p>');
      srv.close();
      if (u.searchParams.get('state') !== state) return reject(new Error('state mismatch'));
      if (u.searchParams.get('error')) return reject(new Error(u.searchParams.get('error')));
      resolve(u.searchParams.get('code'));
    }).listen(8888, '127.0.0.1', () => {
      log('LOGIN NEEDED, opening browser:', url);
      shell.openExternal(url);
    });
  });
  const t = await exchange({ grant_type: 'authorization_code', code, redirect_uri: REDIRECT, client_id: CLIENT_ID, code_verifier: verifier });
  return t.access_token;
}

app.whenReady().then(async () => {
  log('electron', process.versions.electron, 'chrome', process.versions.chrome);
  const token = await getToken();
  log('have token');
  await components.whenReady();
  log('components:', components.status());

  const filter = { urls: ['*://*.spotify.com/*', '*://*.scdn.co/*', '*://*.spotifycdn.com/*'] };
  session.defaultSession.webRequest.onCompleted(filter, d => {
    if (d.url.includes('widevine-license') || d.statusCode >= 400) log('net', d.statusCode, d.method, d.url.slice(0, 160));
  });
  session.defaultSession.webRequest.onErrorOccurred(filter, d => log('neterr', d.error, d.url.slice(0, 160)));

  ipcMain.handle('token', () => token);
  ipcMain.on('log', (_e, ...a) => log('renderer', ...a));
  ipcMain.on('done', () => { log('done'); setTimeout(() => app.quit(), 500); });

  const win = new BrowserWindow({
    width: 480, height: 320,
    webPreferences: { preload: path.join(__dirname, 'preload.js'), contextIsolation: true, nodeIntegration: false },
  });
  win.webContents.on('console-message', (_e, _l, m) => log('console', m));
  win.loadFile('index.html');
});
app.on('window-all-closed', () => app.quit());
process.on('unhandledRejection', e => { log('FATAL', String(e)); app.exit(1); });
