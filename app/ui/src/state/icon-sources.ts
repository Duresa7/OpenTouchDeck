import { useStore } from 'zustand';
import { createStore } from 'zustand/vanilla';

/**
 * The switches for where the icon picker finds icons beyond the bundled ones: installed Icon packs and Brandfetch
 * logos. The Companion keeps no preference for these, so this window keeps them. Turning one off hides its icons
 * and keeps every downloaded file and saved key.
 */
export type IconSource = 'iconPacks' | 'brandfetch';

export interface IconSourcesState {
  /** Null until a default is applied, once, from what's set up. A deliberate off then survives restarts. */
  iconPacks: boolean | null;
  brandfetch: boolean | null;
  set(source: IconSource, on: boolean): void;
  /** Applies the default for a switch nobody has set yet. */
  settle(source: IconSource, on: boolean): void;
}

const KEYS: Record<IconSource, string> = { iconPacks: 'touchdeck.icon-packs-on', brandfetch: 'touchdeck.brandfetch-on' };

type Prefs = Pick<Storage, 'getItem' | 'setItem'>;

function localStorageOrNull(): Prefs | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

function read(storage: Prefs | null, source: IconSource): boolean | null {
  try {
    const value = storage?.getItem(KEYS[source]);
    return value === 'on' ? true : value === 'off' ? false : null;
  } catch {
    return null;
  }
}

function write(storage: Prefs | null, source: IconSource, on: boolean) {
  try {
    storage?.setItem(KEYS[source], on ? 'on' : 'off');
  } catch {
    // Without storage the switch lasts until the window closes.
  }
}

export function createIconSources(storage: Prefs | null = localStorageOrNull()) {
  return createStore<IconSourcesState>()((setState, get) => ({
    iconPacks: read(storage, 'iconPacks'),
    brandfetch: read(storage, 'brandfetch'),
    set: (source, on) => {
      write(storage, source, on);
      setState({ [source]: on });
    },
    settle: (source, on) => {
      if (get()[source] !== null) return;
      get().set(source, on);
    },
  }));
}

export const iconSources = createIconSources();

export function useIconSources<T>(select: (state: IconSourcesState) => T): T {
  return useStore(iconSources, select);
}

/** Whether a switch is on. One nobody has set yet follows what's set up. */
export const sourceOn = (stored: boolean | null, setUp: boolean) => stored ?? setUp;
