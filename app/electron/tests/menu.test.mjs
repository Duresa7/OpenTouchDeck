import { test } from 'node:test';
import assert from 'node:assert/strict';
import { menuTemplate } from '../dist/menu.js';

const roles = (items) => items.flatMap((item) => [item.role, ...(Array.isArray(item.submenu) ? roles(item.submenu) : [])]).filter(Boolean);

test('Windows and Linux keep editing, zoom, full screen, window and quit, and nothing else', () => {
  for (const platform of ['win32', 'linux']) {
    const menu = menuTemplate(platform, true);
    assert.deepEqual(menu.map((item) => item.label), ['File', 'Edit', 'View', 'Window']);
    assert.deepEqual(roles(menu), ['quit', 'undo', 'redo', 'cut', 'copy', 'paste', 'selectAll', 'resetZoom', 'zoomIn', 'zoomOut', 'togglefullscreen', 'minimize', 'close']);
  }
});

test('reload and the developer tools appear only when not packaged', () => {
  assert.equal(roles(menuTemplate('win32', true)).some((role) => role === 'reload' || role === 'toggleDevTools'), false);
  const view = menuTemplate('win32', false).find((item) => item.label === 'View');
  assert.deepEqual(roles(view.submenu).slice(-3), ['reload', 'forceReload', 'toggleDevTools']);
});

test('macOS keeps the same roles under its app menu', () => {
  const menu = menuTemplate('darwin', true);
  assert.equal(menu[0].role, 'appMenu');
  assert.deepEqual(roles(menu.slice(1)), roles(menuTemplate('win32', true).slice(1)));
});
