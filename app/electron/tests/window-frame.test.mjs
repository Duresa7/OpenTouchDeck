import { test } from 'node:test';
import assert from 'node:assert/strict';
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
