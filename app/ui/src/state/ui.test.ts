import { describe, expect, it } from 'vitest';
import { BRANDFETCH, createUiStore, ICON_PACKS, reloadIntoIntegration, reloadIntoSettings, reopenSettings } from './ui.ts';

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
    reloadIntoSettings('troubleshooting', { storage, reload: () => reloads++ });
    expect(reloads).toBe(1);

    const ui = createUiStore(() => storage);
    reopenSettings(ui, storage);
    expect(ui.getState()).toMatchObject({ settingsOpen: true, settingsSection: 'troubleshooting' });

    const again = createUiStore(() => storage);
    reopenSettings(again, storage);
    expect(again.getState().settingsOpen).toBe(false);
  });

  it('ignores a section it does not know and works without storage', () => {
    const storage = memory();
    storage.setItem('touchdeck.reopen-settings', 'elsewhere');
    const ui = createUiStore(() => storage);
    reopenSettings(ui, storage);
    expect(ui.getState().settingsOpen).toBe(false);
    reopenSettings(ui, null);
    let reloads = 0;
    reloadIntoSettings('general', { storage: null, reload: () => reloads++ });
    expect(reloads).toBe(1);
  });
});

describe('Integration settings pages', () => {
  it('open on the Integrations screen and Back returns to the list', () => {
    const ui = createUiStore(() => null);
    ui.getState().openIntegration('cliproxyapi', { setUp: true });
    expect(ui.getState()).toMatchObject({ section: 'integrations', integration: { id: 'cliproxyapi', setUp: true } });
    ui.getState().closeIntegration();
    expect(ui.getState().integration).toBeNull();
  });

  it('Icon packs opens its Integrations page, not Settings', () => {
    const ui = createUiStore(() => null);
    ui.getState().setSettingsOpen(true);
    ui.getState().openIconPacks();
    expect(ui.getState()).toMatchObject({ section: 'integrations', settingsOpen: false, integration: { id: ICON_PACKS } });
  });

  it('keep a pending enable while the page is open, and drop it on Back or on going elsewhere', () => {
    const storage = memory();
    const ui = createUiStore(() => storage);
    ui.getState().openIntegration('cliproxyapi', { setUp: true, pendingEnable: true });
    expect(ui.getState().pendingEnable).toBe('cliproxyapi');
    expect(storage.getItem('touchdeck.pending-enable')).toBe('cliproxyapi');
    ui.getState().closeIntegration();
    expect(ui.getState().pendingEnable).toBeNull();
    expect(storage.getItem('touchdeck.pending-enable')).toBeNull();

    ui.getState().openIntegration('custom', { setUp: true, pendingEnable: true });
    ui.getState().go('layout');
    expect(ui.getState()).toMatchObject({ section: 'layout', integration: null, pendingEnable: null });

    ui.getState().openIntegration('custom', { pendingEnable: true });
    ui.getState().openIntegration('cliproxyapi');
    expect(ui.getState().pendingEnable).toBeNull();
  });

  it('a reload into an Integration page keeps its pending enable, as Brandfetch needs', () => {
    const storage = memory();
    const ui = createUiStore(() => storage);
    ui.getState().openIntegration(BRANDFETCH, { setUp: true, pendingEnable: true });
    let reloads = 0;
    reloadIntoIntegration(BRANDFETCH, { storage, reload: () => reloads++ });
    expect(reloads).toBe(1);

    const after = createUiStore(() => storage);
    reopenSettings(after, storage);
    expect(after.getState()).toMatchObject({ section: 'integrations', integration: { id: BRANDFETCH }, pendingEnable: BRANDFETCH, settingsOpen: false });
  });

  it('a plain reload drops a pending enable left behind', () => {
    const storage = memory();
    storage.setItem('touchdeck.pending-enable', 'cliproxyapi');
    const ui = createUiStore(() => storage);
    reopenSettings(ui, storage);
    expect(ui.getState().pendingEnable).toBeNull();
    expect(storage.getItem('touchdeck.pending-enable')).toBeNull();
  });
});
