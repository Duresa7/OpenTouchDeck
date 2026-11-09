import { useStore } from 'zustand';
import { createStore } from 'zustand/vanilla';
import type { Appearance } from './theme.ts';

export type Section = 'home' | 'layout' | 'integrations' | 'settings';
/** The Settings page's categories, in the order its sidebar lists them. */
export const SETTINGS_SECTIONS = ['general', 'device', 'browsers', 'troubleshooting', 'errors', 'about'] as const;
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number];

/** The Integrations screens that live in this window rather than in the Companion: they feed the icon picker. */
export const ICON_PACKS = 'icon-packs';
export const BRANDFETCH = 'brandfetch';

/** The Integration whose settings page is open, and whether it was opened to finish setting it up. */
export interface OpenIntegration {
  id: string;
  setUp: boolean;
}

export interface UiState {
  section: Section;
  deviceId: string | null;
  settingsSection: SettingsSection;
  /** The Device whose settings the Settings page shows, or null for the one open in the editor. */
  settingsDevice: string | null;
  /** The settings page open on the Integrations screen, or null for the list. */
  integration: OpenIntegration | null;
  /**
   * An Integration whose switch was turned on before it was set up. It turns on once a save sets it up, and is
   * dropped when its settings page is left by Back or by going to another screen.
   */
  pendingEnable: string | null;
  /** The Add Tile tab chosen by hand this session, or null to open on the one that fits. */
  addTileTab: 'actions' | 'integrations' | null;
  setAddTileTab(tab: 'actions' | 'integrations'): void;
  /** Whether the Layout editor's Details panel is out. Picking a Tile or adding one brings it back. */
  detailsOpen: boolean;
  setDetailsOpen(open: boolean): void;
  openDevice(id: string): void;
  openTroubleshooting(): void;
  /** Goes to the Settings page, at a category, and for the device category at a Device. */
  openSettings(section?: SettingsSection, device?: string | null): void;
  openIconPacks(): void;
  /** Opens an Integration's settings page on the Integrations screen. */
  openIntegration(id: string, options?: { setUp?: boolean; pendingEnable?: boolean }): void;
  /** Back to the Integrations list. Drops a pending enable. */
  closeIntegration(): void;
  setPendingEnable(id: string | null): void;
  showSettingsSection(section: SettingsSection): void;
  appearance: Appearance;
  go(section: Section): void;
  setAppearance(appearance: Appearance): void;
}

const PENDING_KEY = 'touchdeck.pending-enable';

function sessionStorageOrNull(): Storage | null {
  try {
    return window.sessionStorage;
  } catch {
    return null;
  }
}

function writePending(storage: Storage | null, id: string | null) {
  try {
    if (id === null) storage?.removeItem(PENDING_KEY);
    else storage?.setItem(PENDING_KEY, id);
  } catch {
    // Without storage a pending enable lasts until the window reloads.
  }
}

export function createUiStore(storage: () => Storage | null = sessionStorageOrNull) {
  return createStore<UiState>()((set, get) => {
    const pending = (id: string | null) => {
      writePending(storage(), id);
      return { pendingEnable: id };
    };
    return {
      section: 'home',
      deviceId: null,
      settingsSection: 'general',
      settingsDevice: null,
      integration: null,
      pendingEnable: null,
      addTileTab: null,
      setAddTileTab: (addTileTab) => set({ addTileTab }),
      detailsOpen: true,
      setDetailsOpen: (detailsOpen) => set({ detailsOpen }),
      openDevice: (deviceId) => set({ section: 'layout', deviceId, integration: null, ...pending(null) }),
      openTroubleshooting: () => get().openSettings('troubleshooting'),
      openSettings: (settingsSection = 'general', device = null) =>
        set({ section: 'settings', settingsSection, settingsDevice: device, integration: null, ...pending(null) }),
      openIconPacks: () => get().openIntegration(ICON_PACKS),
      openIntegration: (id, options = {}) =>
        set({
          section: 'integrations',
          integration: { id, setUp: options.setUp ?? false },
          ...pending(options.pendingEnable ? id : get().pendingEnable === id ? id : null),
        }),
      closeIntegration: () => set({ integration: null, ...pending(null) }),
      setPendingEnable: (id) => set(pending(id)),
      showSettingsSection: (settingsSection) => set({ settingsSection }),
      appearance: 'system',
      go: (section) => set({ section, integration: null, ...pending(null) }),
      setAppearance: (appearance) => set({ appearance }),
    };
  });
}

export const uiStore = createUiStore();

/** Brings the Layout editor's Details panel out: called when the user picks a Tile, adds one, or something needs them there. */
export function openDetails(store: UiStore = uiStore) {
  store.getState().setDetailsOpen(true);
}

export type UiStore = ReturnType<typeof createUiStore>;

const REOPEN_KEY = 'touchdeck.reopen-settings';
const INTEGRATION_PREFIX = 'integration:';

interface Reload {
  storage: Storage | null;
  reload(): void;
}

const pageReload = (): Reload => ({ storage: sessionStorageOrNull(), reload: () => window.location.reload() });

function reloadWith(value: string, target: Reload) {
  try {
    target.storage?.setItem(REOPEN_KEY, value);
  } catch {
    // Without storage the window reloads into Home.
  }
  target.reload();
}

/**
 * Reloads the window and opens Settings at this section again. The Companion sets the page's
 * content policy when the page loads, so a change to it only applies after a reload.
 */
export function reloadIntoSettings(section: SettingsSection, target: Reload = pageReload()) {
  reloadWith(section, target);
}

/** Reloads the window and opens this Integration's settings page again. A pending enable survives the reload. */
export function reloadIntoIntegration(id: string, target: Reload = pageReload()) {
  reloadWith(`${INTEGRATION_PREFIX}${id}`, target);
}

/** Opens Settings, or an Integration's settings page, where a reload asked for, once. */
export function reopenSettings(store: UiStore = uiStore, storage: Storage | null = sessionStorageOrNull()) {
  let section: string | null = null;
  let pendingEnable: string | null = null;
  try {
    section = storage?.getItem(REOPEN_KEY) ?? null;
    storage?.removeItem(REOPEN_KEY);
    pendingEnable = storage?.getItem(PENDING_KEY) ?? null;
  } catch {
    return;
  }
  if (section?.startsWith(INTEGRATION_PREFIX)) {
    const id = section.slice(INTEGRATION_PREFIX.length);
    store.setState({ section: 'integrations', integration: { id, setUp: false }, pendingEnable: pendingEnable === id ? id : null });
    return;
  }
  if (pendingEnable !== null) writePending(storage, null);
  const wanted = SETTINGS_SECTIONS.find((s) => s === section);
  if (wanted) store.setState({ section: 'settings', settingsSection: wanted });
}

export function useUi<T>(select: (state: UiState) => T): T {
  return useStore(uiStore, select);
}
