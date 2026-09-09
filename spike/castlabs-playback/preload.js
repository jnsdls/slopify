const { contextBridge, ipcRenderer } = require('electron');
contextBridge.exposeInMainWorld('spike', {
  token: () => ipcRenderer.invoke('token'),
  log: (...a) => ipcRenderer.send('log', ...a),
  done: () => ipcRenderer.send('done'),
});
