import { app, BrowserWindow, dialog, ipcMain, Menu, shell } from 'electron';
import { spawn } from 'node:child_process';
import { dirname, join } from 'node:path';
import { connect, devUiOrigin, restartMessage, revoke, runtimePath, type WindowSession } from './companion';

import { Updater, feedSettings } from './updates';
import { UPDATER_PUBLIC_KEY } from './updates/key.gen';
import { trustedSender } from './trusted-sender';
import { menuTemplate } from './menu';

app.setName('Open TouchDeck');
if (process.platform === 'win32') {
  app.setAppUserModelId('io.github.duresa7.touchdeck');
  // On Windows 11 22H2 and later, Chromium's TcpPortRandomizationWin makes about one loopback connect in a
  // few thousand fail at once with net::ERR_NO_BUFFER_SPACE. The window loads the UI from the Companion over
  // loopback, so one such failure leaves a blank page or a failed load and the error box. Nothing else sets
  // --disable-features here; Chromium would keep only the last one.
  app.commandLine.appendSwitch('disable-features', 'TcpPortRandomizationWin');
}
if (process.env.TOUCHDECK_APP_DATA_DIR) app.setPath('userData', join(process.env.TOUCHDECK_APP_DATA_DIR, 'electron'));
let window: BrowserWindow | undefined;
let current: WindowSession | undefined;
let closing = false;
let refreshing: Promise<string | null> | undefined;
// Development only: show the UI dev server, which proxies /api to the Companion and mints its own session,
// in place of the Companion's page. It never gets the window session: `session` answers only the Companion origin.
const devUi = app.isPackaged ? undefined : devUiOrigin(process.env.TOUCHDECK_UI_DEV_URL);

/** The origin the window shows and may navigate within. */
function pageOrigin(session: WindowSession): string {
  return devUi ?? session.origin;
}

function startCompanion() {
  const binary = app.isPackaged
    ? join(dirname(app.getPath('exe')), process.platform === 'win32' ? 'td-companion.exe' : 'td-companion')
    : process.env.TOUCHDECK_COMPANION_BIN;
  if (!binary) throw new Error('Set TOUCHDECK_COMPANION_BIN to the Companion binary');
  const child = spawn(binary, app.isPackaged ? ['--packaged'] : [], { detached: true, stdio: 'ignore', windowsHide: true });
  child.on('error', () => { /* The bounded runtime-file wait reports launch failure. */ });
  child.unref();
}

function trusted(event: Electron.IpcMainInvokeEvent): boolean {
  const frame = event.senderFrame;
  return trustedSender({ sender: event.sender, frame, origin: frame?.origin, window, closing, expectedOrigin: current?.origin });
}

function sameOrigin(url: string): boolean {
  try { return !!current && new URL(url).origin === pageOrigin(current); } catch { return false; }
}

function external(url: string) {
  try {
    if (['https:', 'http:', 'mailto:'].includes(new URL(url).protocol)) void shell.openExternal(url).catch(() => {});
  } catch { /* Malformed URLs are blocked. */ }
}

async function refresh(): Promise<string | null> {
  const next = await connect(runtimePath(), startCompanion);
  if (closing || !window || window.isDestroyed()) {
    await revoke(next).catch(() => {});
    return null;
  }
  const previous = current;
  current = next;
  if (previous) await revoke(previous).catch(() => {});
  if (window.webContents.mainFrame.origin !== pageOrigin(next)) {
    await window.loadURL(pageOrigin(next));
    return null;
  }
  window.webContents.send('session-changed', next.token);
  return next.token;
}

ipcMain.handle('session', async (event, stale: unknown) => {
  if (!trusted(event)) return null;
  if (!refreshing && (typeof stale !== 'string' || stale !== current?.token)) return current?.token ?? null;
  refreshing ??= refresh().finally(() => { refreshing = undefined; });
  const token = await refreshing;
  return trusted(event) ? token : null;
});

ipcMain.handle('dialog:pick-folder', async event => {
  if (!trusted(event) || !window) return null;
  try {
    const picked = await dialog.showOpenDialog(window, { title: 'Choose a folder for the photo frame', properties: ['openDirectory'] });
    return trusted(event) && !picked.canceled ? picked.filePaths[0] ?? null : null;
  } catch { return null; }
});

let updater: Updater | undefined;
for (const action of ['check', 'download', 'install'] as const) {
  ipcMain.handle(`updates:${action}`, async event => {
    const failed = action === 'check' ? { state: 'failed' } : { state: 'failed', reason: action === 'download' ? 'download' : 'installer' };
    if (!trusted(event) || !updater) return failed;
    try {
      if (action === 'download') return await updater.download(progress => {
        if (trusted(event)) event.senderFrame?.send('updates:progress', progress);
      });
      return await updater[action]();
    } catch { return failed; }
  });
}

async function openWindow() {
  current = await connect(runtimePath(), startCompanion);
  updater = new Updater({
    publicKey: UPDATER_PUBLIC_KEY, current: app.getVersion(), runtimeDir: dirname(runtimePath()),
    ...feedSettings(app.isPackaged),
    system: () => {
      const session = current;
      if (!session) throw new Error('Companion unavailable');
      return async (method, path) => {
        const res = await fetch(`${session.origin}/api/v1${path}`, {
          method, redirect: 'error', signal: AbortSignal.timeout(5000),
          headers: { Authorization: `Bearer ${session.token}`, 'Content-Type': 'application/json' },
        });
        return { status: res.status, body: await res.json().catch(() => null) };
      };
    },
    launch: (path, args) => new Promise((resolve, reject) => {
      const child = spawn(path, args, { detached: true, stdio: 'ignore', windowsHide: true });
      child.once('error', reject);
      child.once('spawn', () => { child.unref(); resolve(); });
    }),
    quit: () => { closing = true; app.quit(); },
  });
  Menu.setApplicationMenu(Menu.buildFromTemplate(menuTemplate(process.platform, app.isPackaged)));
  window = new BrowserWindow({
    title: 'Open TouchDeck', useContentSize: true, width: 1100, height: 760, minWidth: 800, minHeight: 600, autoHideMenuBar: true,
    webPreferences: { preload: join(__dirname, 'preload.js'), contextIsolation: true, sandbox: true,
      nodeIntegration: false, webviewTag: false, partition: 'touchdeck-window' },
  });
  window.webContents.session.setPermissionRequestHandler((_contents, _permission, callback) => callback(false));
  window.webContents.session.setPermissionCheckHandler(() => false);
  window.on('page-title-updated', event => event.preventDefault());
  const guard = (event: Electron.Event, url: string) => {
    if (!sameOrigin(url)) { event.preventDefault(); external(url); }
  };
  window.webContents.on('will-navigate', guard);
  window.webContents.on('will-redirect', guard);
  window.webContents.setWindowOpenHandler(({ url }) => {
    if (sameOrigin(url)) void window?.loadURL(url);
    else external(url);
    return { action: 'deny' };
  });
  window.on('close', event => {
    if (closing) return;
    event.preventDefault();
    closing = true;
    void (async () => {
      await refreshing?.catch(() => {});
      if (current) await revoke(current).catch(() => {});
      window?.destroy();
      app.quit();
    })();
  });
  await window.loadURL(pageOrigin(current));
}

if (!app.requestSingleInstanceLock()) app.quit();
else {
  app.on('second-instance', () => {
    if (window?.isMinimized()) window.restore();
    window?.show(); window?.focus();
  });
  app.on('window-all-closed', () => app.quit());
  void app.whenReady().then(openWindow).catch(error => {
    if (closing) return;
    // The dialog blocks until someone dismisses it; the log says why the window never opened.
    console.error('Open TouchDeck could not open its window:', error);
    dialog.showErrorBox('Open TouchDeck', error instanceof Error && error.message === restartMessage
      ? restartMessage : 'Could not verify or start the Companion. Try restarting Open TouchDeck.');
    app.quit();
  });
}
