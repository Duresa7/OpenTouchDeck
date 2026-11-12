import type { MenuItemConstructorOptions } from 'electron';

/**
 * The window's menu: only what people use from it. Edit keeps undo, redo and the clipboard shortcuts working, View
 * keeps zoom and full screen, and Window and quit close things. Reload and the developer tools are only in a build
 * that isn't packaged. On Windows and Linux the window draws no menu bar, so only the shortcuts remain; macOS keeps its
 * app menu.
 */
export function menuTemplate(platform: NodeJS.Platform, packaged: boolean): MenuItemConstructorOptions[] {
  const develop: MenuItemConstructorOptions[] = packaged ? [] : [{ type: 'separator' }, { role: 'reload' }, { role: 'forceReload' }, { role: 'toggleDevTools' }];
  const edit: MenuItemConstructorOptions = {
    label: 'Edit',
    submenu: [{ role: 'undo' }, { role: 'redo' }, { type: 'separator' }, { role: 'cut' }, { role: 'copy' }, { role: 'paste' }, { role: 'selectAll' }],
  };
  const view: MenuItemConstructorOptions = {
    label: 'View',
    submenu: [{ role: 'resetZoom' }, { role: 'zoomIn' }, { role: 'zoomOut' }, { type: 'separator' }, { role: 'togglefullscreen' }, ...develop],
  };
  const window: MenuItemConstructorOptions = { label: 'Window', submenu: [{ role: 'minimize' }, { role: 'close' }] };
  if (platform === 'darwin') return [{ role: 'appMenu' }, edit, view, window];
  return [{ label: 'File', submenu: [{ role: 'quit' }] }, edit, view, window];
}
