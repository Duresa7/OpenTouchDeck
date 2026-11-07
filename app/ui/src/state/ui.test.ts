import { describe, expect, it } from 'vitest';
import { createUiStore, reloadIntoSettings, reopenSettings } from './ui.ts';

function memory(): Storage {
  const items = new Map<string, string>();
  return {
    get length() {
      return items.size;
    },
    clear: () => items.clear(),
    getItem: (key) => items.get(key) ?? null,
    key: (index) => [...items.keys()][index] ?? null,
    removeItem: (key) => void items.delete(key),
    setItem: (key, value) => void items.set(key, value),
  };
}

describe('a reload into Settings', () => {
  it('opens Settings at the same section once', () => {
    const storage = memory();
    let reloads = 0;
    reloadIntoSettings('brandfetch', { storage, reload: () => reloads++ });
    expect(reloads).toBe(1);

    const ui = createUiStore();
    reopenSettings(ui, storage);
    expect(ui.getState()).toMatchObject({ settingsOpen: true, settingsSection: 'brandfetch' });

    const again = createUiStore();
    reopenSettings(again, storage);
    expect(again.getState().settingsOpen).toBe(false);
  });

  it('ignores a section it does not know and works without storage', () => {
    const storage = memory();
    storage.setItem('touchdeck.reopen-settings', 'elsewhere');
    const ui = createUiStore();
    reopenSettings(ui, storage);
    expect(ui.getState().settingsOpen).toBe(false);
    reopenSettings(ui, null);
    let reloads = 0;
    reloadIntoSettings('general', { storage: null, reload: () => reloads++ });
    expect(reloads).toBe(1);
  });
});
