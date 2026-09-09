import { Tray, nativeImage } from 'electron';
import { resourcePath } from './resources';

let tray: Tray | null = null;

// The "Template" suffix makes macOS tint the image; createFromPath picks up the @2x sibling itself.
const icon = (attention: boolean) => nativeImage.createFromPath(resourcePath(attention ? 'trayAttentionTemplate.png' : 'trayTemplate.png'));

export function createTray({ onToggle }: { onToggle: () => void }): Tray {
  tray = new Tray(icon(false));
  tray.setIgnoreDoubleClickEvents(true);
  tray.on('click', onToggle);
  tray.on('right-click', onToggle);
  return tray;
}

export function setAttention(on: boolean): void {
  tray?.setImage(icon(on));
}
