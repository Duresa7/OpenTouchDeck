import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { frameOptions, overlayColors } from '../dist/window-frame.js';

test('Windows and Linux overlay the native caption buttons without removing the frame', () => {
  for (const platform of ['win32', 'linux']) {
    const options = frameOptions(platform, false);
    assert.equal(options.titleBarStyle, 'hidden');
    assert.deepEqual(options.titleBarOverlay, { ...overlayColors(false), height: 32 });
    assert.equal(options.frame, undefined);
    assert.notDeepEqual(overlayColors(true), overlayColors(false));
  }
  assert.deepEqual(frameOptions('darwin', false), {});
});

test('the top strip uses the reported safe rectangle, drags, and excludes controls', () => {
  const css = readFileSync(new URL('../../ui/src/shell/window-frame.css', import.meta.url), 'utf8');
  for (const dimension of ['x', 'y', 'width', 'height']) assert.ok(css.includes(`env(titlebar-area-${dimension},`));
  assert.match(css, /-webkit-app-region: drag/);
  assert.match(css, /:is\(button, input, select, textarea, a,.*[\s\S]*-webkit-app-region: no-drag/);
  assert.match(css, /\.window-drag-strip\s*{\s*display: none/);
});
