import { app } from 'electron';
import path from 'node:path';

/** electron-builder's extraResources copies `resources/` into the bundle's Resources dir. */
export function resourcePath(name: string): string {
  const root = app.isPackaged ? path.join(process.resourcesPath, 'resources') : path.join(app.getAppPath(), 'resources');
  return path.join(root, name);
}
