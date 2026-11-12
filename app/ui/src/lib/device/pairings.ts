import { createStore } from 'zustand/vanilla';
import type { DevicePairings, DeviceSummary } from '../companion/api.ts';
import type { Companion } from '../companion/client.ts';
import { toApiError } from '../companion/errors.ts';

// The apps a radio-build Device keeps Pairings with (docs/companion-api.md, devices.pairings_*): the list, and
// forgetting one or every one. Every request goes over USB only. No answer carries a key.

/** `true` for a Device whose Firmware keeps Pairings: the radio build, which has a Wi-Fi station. */
export function offersPairings(device: Pick<DeviceSummary, 'capabilities'>): boolean {
  return device.capabilities?.wifi === true;
}

/** The count line, as "2 of 4". */
export const pairedCount = (pairings: DevicePairings) => `${pairings.pairings.length} of ${pairings.max}`;

export interface DevicePairingsState {
  load: 'idle' | 'loading' | 'ready' | 'failed';
  pairings: DevicePairings | null;
  /** The app being forgotten, or `all`. */
  forgetting: string | null;
  /** Firmware without Pairings answered. */
  needs: boolean;
  error: string | null;
}

type Summary = Pick<DeviceSummary, 'id' | 'name' | 'link' | 'capabilities' | 'carrier'>;

export interface DevicePairingsDeps {
  companion: Pick<Companion, 'devices'>;
  device: Summary;
  /** Called after a forget, answered or not: this app's own Pairing may have changed. */
  changed?: () => void;
}

const UNKNOWN = new Set(['device.timeout', 'device.unknown_outcome']);

export function createDevicePairings(deps: DevicePairingsDeps) {
  const { companion, changed = () => {} } = deps;
  let device = deps.device;
  const store = createStore<DevicePairingsState>()(() => ({ load: 'idle', pairings: null, forgetting: null, needs: false, error: null }));
  const { getState: state, setState: set } = store;
  const name = () => device.name ?? 'the Device';
  // The Device lists and forgets Pairings only over USB, so nothing is sent while it is on Wi-Fi.
  const overUsb = () => device.link.state === 'connected' && device.carrier !== 'wifi';
  let seq = 0;

  function failed(thrown: unknown) {
    const error = toApiError(thrown);
    switch (error.code) {
      case 'device.needs_firmware':
        set({ needs: true, error: null });
        return;
      case 'device.usb_only':
        set({ error: `Plug in ${name()} with a USB cable to change its paired apps.` });
        return;
      case 'pairings.storage':
        set({ error: `${name()} couldn't erase that. Try again.` });
        return;
      default:
        set({
          error: UNKNOWN.has(error.code)
            ? `${name()} didn't answer, so the app may still be paired. Plug ${name()} in again to finish.`
            : `Couldn't reach ${name()}. Check that it's plugged in, then retry.`,
        });
    }
  }

  /** Reads the list. `report` false keeps a failure quiet, so the error on show stays. */
  async function read(report = true): Promise<boolean> {
    const mine = ++seq;
    try {
      const pairings = await companion.devices.pairings_get({ device: device.id });
      if (mine === seq) set({ pairings, load: 'ready', needs: false });
      return true;
    } catch (thrown) {
      if (mine !== seq) return false;
      if (state().load === 'loading') set({ load: 'failed' });
      if (report) failed(thrown);
      return false;
    }
  }

  async function run(forgetting: string, request: () => Promise<DevicePairings>): Promise<boolean> {
    if (state().forgetting || !overUsb() || !offersPairings(device)) return false;
    const mine = ++seq;
    set({ forgetting, error: null });
    try {
      const pairings = await request();
      set({ forgetting: null });
      if (mine === seq) set({ pairings, load: 'ready' });
      return true;
    } catch (thrown) {
      set({ forgetting: null });
      failed(thrown);
      // The Companion sends an unanswered forget again before the next list, so one read may finish it.
      if (UNKNOWN.has(toApiError(thrown).code) && (await read(false))) set({ error: null });
      return false;
    } finally {
      changed();
    }
  }

  const api = {
    async load(): Promise<void> {
      if (!overUsb() || !offersPairings(device)) return;
      if (state().load !== 'ready') set({ load: 'loading' });
      if (await read()) set({ error: null });
    },

    forget(app: string): Promise<boolean> {
      return run(app, () => companion.devices.pairings_forget({ device: device.id, app }));
    },

    forgetAll(): Promise<boolean> {
      return run('all', () => companion.devices.pairings_forget_all({ device: device.id }));
    },

    /** How many apps the Device keeps, once read over USB. */
    count(): number | null {
      return state().pairings?.pairings.length ?? null;
    },

    observe(summary: Summary) {
      const wasUsb = overUsb();
      device = summary;
      if (!wasUsb && overUsb()) void api.load();
    },

    dispose() {
      seq += 1;
    },
  };

  return Object.assign(store, api);
}

export type DevicePairingsStore = ReturnType<typeof createDevicePairings>;
