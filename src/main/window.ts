import { BrowserWindow, screen, type Tray } from 'electron';
import path from 'node:path';
import { ipc } from '../shared/bridge';

const WIDTH = 300;
const MIN_HEIGHT = 120;
const MAX_HEIGHT = 560;
const TOGGLE_DEBOUNCE_MS = 200;

let win: BrowserWindow | null = null;
let height = 400;
let quitting = false;
let hiddenAt = 0;

export function createWindow(): BrowserWindow {
  win = new BrowserWindow({
    width: WIDTH,
    height,
    show: false,
    frame: false,
    resizable: false,
    movable: false,
    minimizable: false,
    fullscreenable: false,
    alwaysOnTop: true,
    skipTaskbar: true,
    hiddenInMissionControl: true,
    transparent: false,
    webPreferences: {
      preload: path.join(import.meta.dirname, '../preload/index.cjs'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      autoplayPolicy: 'no-user-gesture-required',
    },
  });
  win.setVisibleOnAllWorkspaces(true, { visibleOnFullScreen: true });
  win.on('blur', hideWindow);
  // The Player lives here; closing would kill playback.
  win.on('close', (event) => {
    if (quitting) return;
    event.preventDefault();
    hideWindow();
  });

  if (process.env.ELECTRON_RENDERER_URL) void win.loadURL(process.env.ELECTRON_RENDERER_URL);
  else void win.loadFile(path.join(import.meta.dirname, '../renderer/index.html'));
  return win;
}

export function getWindow(): BrowserWindow | null {
  return win;
}

export function markQuitting(): void {
  quitting = true;
}

export function setContentHeight(px: number): void {
  if (!win || !Number.isFinite(px)) return;
  height = Math.round(Math.min(MAX_HEIGHT, Math.max(MIN_HEIGHT, px)));
  const { x, y } = win.getBounds();
  win.setBounds(clampToWorkArea({ x, y, width: WIDTH, height }));
}

export function showWindow(tray: Tray): void {
  if (!win) return;
  win.setBounds(positionUnder(tray.getBounds()));
  win.show();
  win.focus();
  win.webContents.send(ipc.windowShown, true);
}

export function hideWindow(): void {
  if (!win?.isVisible()) return;
  hiddenAt = Date.now();
  win.hide();
  win.webContents.send(ipc.windowShown, false);
}

export function toggleWindow(tray: Tray): void {
  if (!win) return;
  // Clicking the tray while shown fires blur (hide) before click; do not re-show on that click.
  if (win.isVisible()) hideWindow();
  else if (Date.now() - hiddenAt > TOGGLE_DEBOUNCE_MS) showWindow(tray);
}

function positionUnder(tray: Electron.Rectangle): Electron.Rectangle {
  const x = Math.round(tray.x + tray.width / 2 - WIDTH / 2);
  const y = Math.round(tray.y + tray.height);
  return clampToWorkArea({ x, y, width: WIDTH, height });
}

function clampToWorkArea(bounds: Electron.Rectangle): Electron.Rectangle {
  const area = screen.getDisplayNearestPoint({ x: bounds.x, y: bounds.y }).workArea;
  const h = Math.min(bounds.height, area.height);
  return {
    x: Math.max(area.x, Math.min(bounds.x, area.x + area.width - bounds.width)),
    y: Math.max(area.y, Math.min(bounds.y, area.y + area.height - h)),
    width: bounds.width,
    height: h,
  };
}
