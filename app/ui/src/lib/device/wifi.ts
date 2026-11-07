import { createStore, type StoreApi } from 'zustand/vanilla';
import type { DeviceSummary, PairingStarted, WifiPairing } from '../companion/api.ts';
import type { Companion } from '../companion/client.ts';
import { toApiError, type ApiError } from '../companion/errors.ts';

// Pairing one Device over Wi-Fi (docs/companion-api.md, Wi-Fi Carrier): find it on the
// network or take a typed address, start pairing, then send the six digits the Device shows.

export type WifiStep =
  | { step: 'idle' }
  | { step: 'finding' }
  | { step: 'address'; found: boolean }
  | { step: 'starting' }
  | { step: 'code'; attempt: string; address: string; attemptsLeft: number }
  | { step: 'checking'; attempt: string; address: string; attemptsLeft: number };

export interface WifiState {
  loaded: 'idle' | 'loading' | 'ready' | 'failed';
  /** This Device's Pairing, or null when it has none. */
  pairing: WifiPairing | null;
  view: WifiStep;
  /** The address field: what discovery found, or what the person typed. */
  address: string;
  problem: string | null;
}

export interface WifiActions {
  load(): Promise<void>;
  /** Opens the flow: a short network search, then the address field. */
  begin(): Promise<void>;
  type(address: string): void;
  start(): Promise<void>;
  submit(code: string): Promise<void>;
  cancel(): Promise<void>;
  forget(): Promise<void>;
}

export type WifiStore = StoreApi<WifiState & WifiActions>;

export interface WifiDeps {
  companion: Pick<Companion, 'wifi'>;
  /** The Device, read on each use so a rename shows in the messages. */
  device: () => Pick<DeviceSummary, 'id' | 'name'>;
}

export const CODE_DIGITS = 6;

export const isCode = (code: string) => new RegExp(`^\\d{${CODE_DIGITS}}$`).test(code);

/** The Carrier in words, or null when the Device isn't connected. */
export function carrierText(device: Pick<DeviceSummary, 'link' | 'carrier'>): string | null {
  if (device.link.state !== 'connected') return null;
  switch (device.carrier) {
    case 'wifi':
      return `Connected over Wi-Fi at ${device.link.port}`;
    case 'usb':
      return 'Connected over USB';
    case null:
      return null;
    default: {
      const _exhaustive: never = device.carrier;
      return _exhaustive;
    }
  }
}

function attemptsText(left: number) {
  return left === 1 ? '1 try left' : `${left} tries left`;
}

function attemptsLeft(details: unknown): number | null {
  if (typeof details !== 'object' || details === null || !('attempts_left' in details)) return null;
  const left = details.attempts_left;
  return typeof left === 'number' ? left : null;
}

/** What to tell the person for each `wifi.*` error. */
export function wifiProblem(error: ApiError, name: string): string {
  switch (error.code) {
    case 'wifi.unreachable':
      return `Nothing answered at that address. Check that ${name} is on the same network, and the address.`;
    case 'wifi.window_closed':
      return `${name} isn't pairing. On ${name}, open Settings and choose Pair new app, then try again.`;
    case 'wifi.wrong_code': {
      const left = attemptsLeft(error.details);
      return left === null ? 'Wrong code.' : `Wrong code. ${attemptsText(left)}.`;
    }
    case 'wifi.locked':
      return `Too many wrong codes. ${name} stopped pairing for a while. Wait, then start again.`;
    case 'wifi.expired':
      return `Pairing ended on ${name}. Start again.`;
    case 'wifi.full':
      return `${name} is paired with four apps. Forget one on ${name}, then try again.`;
    case 'wifi.already_paired':
      return `${name} is already paired over Wi-Fi.`;
    case 'wifi.busy':
      return `${name} is busy with another app. Close it, then try again.`;
    case 'wifi.attempt_not_found':
      return 'That pairing ended. Start again.';
    case 'wifi.keychain':
    case 'wifi.discovery':
    case 'wifi.failed':
      return error.message;
    default:
      return error.message;
  }
}

/** Whether the error ends the attempt, so the flow goes back to the address field. */
const endsAttempt = (code: string) => code !== 'wifi.wrong_code' && code !== 'wifi.keychain';

export function createWifiPairing({ companion, device: current }: WifiDeps): WifiStore {
  const id = current().id;
  const nameOf = () => current().name ?? 'the Device';
  return createStore<WifiState & WifiActions>()((set, get) => {
    const fail = (error: unknown) => set({ problem: wifiProblem(toApiError(error), nameOf()) });

    return {
      loaded: 'idle',
      pairing: null,
      view: { step: 'idle' },
      address: '',
      problem: null,

      async load() {
        if (get().loaded === 'loading') return;
        set({ loaded: 'loading' });
        try {
          const pairings = await companion.wifi.pairings_list();
          set({ loaded: 'ready', pairing: pairings.find((p) => p.device === id) ?? null });
        } catch {
          set({ loaded: 'failed' });
        }
      },

      async begin() {
        if (get().view.step !== 'idle') return;
        set({ view: { step: 'finding' }, problem: null });
        let address = '';
        try {
          const found = await companion.wifi.discover();
          address = found.find((d) => d.device === id)?.address ?? '';
        } catch (error) {
          fail(error);
        }
        if (get().view.step === 'finding') set({ view: { step: 'address', found: address !== '' }, address });
      },

      type(address) {
        set({ address, problem: null });
      },

      async start() {
        const address = get().address.trim();
        if (!address || get().view.step !== 'address') return;
        set({ view: { step: 'starting' }, problem: null });
        let started: PairingStarted;
        try {
          started = await companion.wifi.pair_start({}, null, { address });
        } catch (error) {
          set({ view: { step: 'address', found: false } });
          fail(error);
          return;
        }
        if (started.device !== id) {
          await companion.wifi.pair_cancel({ attempt: started.attempt }).catch(() => undefined);
          set({ view: { step: 'address', found: false }, problem: `Another Device answered at that address, not ${nameOf()}.` });
          return;
        }
        set({ view: { step: 'code', attempt: started.attempt, address: started.address, attemptsLeft: started.attempts_left } });
      },

      async submit(code) {
        const view = get().view;
        if (view.step !== 'code') return;
        if (!isCode(code)) {
          set({ problem: `Enter the ${CODE_DIGITS} digits ${nameOf()} shows.` });
          return;
        }
        set({ view: { ...view, step: 'checking' }, problem: null });
        try {
          const pairing = await companion.wifi.pair_code({ attempt: view.attempt }, null, { code });
          set({ view: { step: 'idle' }, pairing, address: '' });
        } catch (thrown) {
          const error = toApiError(thrown);
          const left = attemptsLeft(error.details);
          set({
            view: endsAttempt(error.code) ? { step: 'address', found: false } : { ...view, attemptsLeft: left ?? view.attemptsLeft },
            problem: wifiProblem(error, nameOf()),
          });
        }
      },

      async cancel() {
        const view = get().view;
        set({ view: { step: 'idle' }, problem: null, address: '' });
        if (view.step === 'code' || view.step === 'checking') await companion.wifi.pair_cancel({ attempt: view.attempt }).catch(() => undefined);
      },

      async forget() {
        const pairing = get().pairing;
        if (!pairing) return;
        try {
          await companion.wifi.pairings_forget({ device: pairing.device });
          set({ pairing: null, problem: null });
        } catch (error) {
          fail(error);
        }
      },
    };
  });
}
