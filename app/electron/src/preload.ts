import { contextBridge, ipcRenderer } from 'electron';

import type { UpdateCheck, UpdateDownload, UpdateInstall, UpdateProgress } from './updates/types';
let downloading = false;
contextBridge.exposeInMainWorld('openTouchDeck', {
  version: 1,
  session: (stale?: string): Promise<string | null> => ipcRenderer.invoke('session', stale),
  onSessionChanged: (callback: (token: string) => void) => {
    if (typeof callback !== 'function') throw new TypeError('Expected a callback');
    const listener = (_event: Electron.IpcRendererEvent, token: string) => callback(token);
    ipcRenderer.on('session-changed', listener);
    return () => ipcRenderer.removeListener('session-changed', listener);
  },
  updates: {
    check: (): Promise<UpdateCheck> => ipcRenderer.invoke('updates:check').catch(() => ({ state: 'failed' })),
    download: async (onProgress: (p: UpdateProgress) => void): Promise<UpdateDownload> => {
      if (downloading) return { state: 'failed', reason: 'download' };
      downloading = true;
      const listener = (_event: Electron.IpcRendererEvent, progress: UpdateProgress) => {
        try { if (typeof onProgress === 'function') onProgress(progress); } catch { }
      };
      ipcRenderer.on('updates:progress', listener);
      try { return await ipcRenderer.invoke('updates:download'); }
      catch { return { state: 'failed', reason: 'download' }; }
      finally { downloading = false; ipcRenderer.removeListener('updates:progress', listener); }
    },
    install: (): Promise<UpdateInstall> => ipcRenderer.invoke('updates:install').catch(() => ({ state: 'failed', reason: 'installer' })),
  },
  pickFolder: (): Promise<string | null> => ipcRenderer.invoke('dialog:pick-folder').catch(() => null),
});

// Observe the same attribute that drives CSS. No general-purpose IPC is exposed to the page.
window.addEventListener('DOMContentLoaded', () => {
  const update = () => ipcRenderer.send('window:appearance', document.documentElement.dataset.theme ?? 'system');
  new MutationObserver(update).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
  update();
});
