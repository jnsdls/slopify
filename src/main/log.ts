import { app } from 'electron';
import log from 'electron-log/main';
import path from 'node:path';

log.transports.file.resolvePathFn = () => path.join(app.getPath('userData'), 'logs', 'slopify.log');
log.transports.file.maxSize = 1024 * 1024;
log.transports.console.level = app.isPackaged ? false : 'info';

export { log };
