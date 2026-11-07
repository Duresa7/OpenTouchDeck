import { useStore } from 'zustand';
import { createStore } from 'zustand/vanilla';
import type { Appearance } from './theme.ts';

export type Section = 'home' | 'layout' | 'integrations';
export type SettingsSection = 'general' | 'troubleshooting' | 'icon-packs' | 'brandfetch';

export interface UiState {
  section: Section;
  settingsOpen: boolean;
  deviceId: string | null;
  settingsSection: SettingsSection;
  openDevice(id: string): void;
  openTroubleshooting(): void;
  openIconPacks(): void;
  showSettingsSection(section: SettingsSection): void;
  appearance: Appearance;
  go(section: Section): void;
  setSettingsOpen(open: boolean): void;
  setAppearance(appearance: Appearance): void;
}

export function createUiStore() {
  return createStore<UiState>()((set) => ({
    section: 'home',
    deviceId: null,
    settingsSection: 'general',
    openDevice: (deviceId) => set({ section: 'layout', deviceId }),
    openTroubleshooting: () => set({ settingsOpen: true, settingsSection: 'troubleshooting' }),
    openIconPacks: () => set({ settingsOpen: true, settingsSection: 'icon-packs' }),
    showSettingsSection: (settingsSection) => set({ settingsSection }),
    settingsOpen: false,
    appearance: 'system',
    go: (section) => set({ section }),
    setSettingsOpen: (settingsOpen) => set(settingsOpen ? { settingsOpen, settingsSection: 'general' } : { settingsOpen }),
    setAppearance: (appearance) => set({ appearance }),
  }));
}

export const uiStore = createUiStore();

export type UiStore = ReturnType<typeof createUiStore>;

const REOPEN_KEY = 'touchdeck.reopen-settings';
const SECTIONS: readonly SettingsSection[] = ['general', 'troubleshooting', 'icon-packs', 'brandfetch'];

function sessionStorageOrNull(): Storage | null {
  try {
    return window.sessionStorage;
  } catch {
    return null;
  }
}

/**
 * Reloads the window and opens Settings at this section again. The Companion sets the page's
 * content policy when the page loads, so a change to it only applies after a reload.
 */
export function reloadIntoSettings(section: SettingsSection, target: { storage: Storage | null; reload(): void } = { storage: sessionStorageOrNull(), reload: () => window.location.reload() }) {
  try {
    target.storage?.setItem(REOPEN_KEY, section);
  } catch {
    // Without storage the window reloads into Home.
  }
  target.reload();
}

/** Opens Settings at the section a reload asked for, once. */
export function reopenSettings(store: UiStore = uiStore, storage: Storage | null = sessionStorageOrNull()) {
  let section: string | null = null;
  try {
    section = storage?.getItem(REOPEN_KEY) ?? null;
    storage?.removeItem(REOPEN_KEY);
  } catch {
    return;
  }
  const wanted = SECTIONS.find((s) => s === section);
  if (wanted) store.setState({ settingsOpen: true, settingsSection: wanted });
}

export function useUi<T>(select: (state: UiState) => T): T {
  return useStore(uiStore, select);
}
