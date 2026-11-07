import { createStore } from 'zustand/vanilla';
import type { DeviceSummary, DeviceWifi } from '../companion/api.ts';
import type { Companion } from '../companion/client.ts';
import { toApiError, type ApiError } from '../companion/errors.ts';

// The Device's Wi-Fi network, for a Device whose Firmware has a Wi-Fi station (the radio build). The store keeps
// what the Device reports and never the password: `join` hands it straight to the Companion and forgets it.

export const SSID_BYTES = 32;
export const PASSWORD_MIN = 8;
export const PASSWORD_MAX = 64;

const bytes = (text: string) => new TextEncoder().encode(text).length;
const special = (text: string) => [...text].some((c) => c <= '\u001f' || c === '\u007f');

/** `true` for a Device that advertises a Wi-Fi station. */
export function offersWifi(device: Pick<DeviceSummary, 'capabilities'>): boolean {
  return device.capabilities?.wifi === true;
}

export function ssidProblem(ssid: string): string | null {
  if (ssid.length === 0) return 'Type the network name.';
  if (bytes(ssid) > SSID_BYTES) return 'That network name is too long.';
  if (special(ssid)) return 'Remove tabs, line breaks and other special characters.';
  return null;
}

/** Empty for an open network, 8 to 63 characters, or 64 hex digits. */
export function passwordProblem(password: string): string | null {
  if (password.length === 0) return null;
  const length = bytes(password);
  if (length === PASSWORD_MAX && /^[0-9a-f]+$/iu.test(password)) return null;
  if (length < PASSWORD_MIN) return 'A Wi-Fi password has at least 8 characters.';
  if (length >= PASSWORD_MAX) return 'A Wi-Fi password has at most 63 characters, or exactly 64 hex digits.';
  if (special(password)) return 'Remove tabs, line breaks and other special characters.';
  return null;
}

/** Why the station failed, in words, from the reason the Device reports. */
export function wifiReasonText(reason: string | null, ssid: string, device: string): string {
  switch (reason) {
    case 'not-found':
      return `Can't find ${ssid}. Check the name, and that ${device} is in range.`;
    case 'wrong-password':
      return `${ssid} refused the password.`;
    case 'no-ip':
      return `Joined ${ssid}, but it gave ${device} no address.`;
    case 'security':
      return `${ssid} uses security ${device} doesn't support, such as WEP or WPA Enterprise.`;
    case 'refused':
      return `${ssid} turned ${device} away. It may be full.`;
    case 'lost':
      return `Lost ${ssid}.`;
    case 'driver':
      return `${device}'s Wi-Fi didn't start. Restart ${device}.`;
    default:
      return `Couldn't join ${ssid}.`;
  }
}

/** One line on how the station is doing. */
export function wifiStatusText(wifi: DeviceWifi, device: string): string {
  switch (wifi.state) {
    case 'off':
      return `No network. ${device} works over USB without one.`;
    case 'connecting':
      return `Joining ${wifi.ssid}…`;
    case 'connected':
      return wifi.ip ? `Connected to ${wifi.ssid} at ${wifi.ip}` : `Connected to ${wifi.ssid}`;
    case 'failed':
      return `${wifiReasonText(wifi.reason, wifi.ssid, device)} ${device} keeps trying.`;
    default: {
      const _exhaustive: never = wifi.state;
      return _exhaustive;
    }
  }
}

export interface DeviceWifiState {
  load: 'idle' | 'loading' | 'ready' | 'failed';
  wifi: DeviceWifi | null;
  busy: 'joining' | 'forgetting' | null;
  problem: { ssid?: string; password?: string };
  /** Firmware without a Wi-Fi station answered. */
  needs: boolean;
  error: string | null;
}

export interface DeviceWifiDeps {
  companion: Pick<Companion, 'devices'>;
  device: Pick<DeviceSummary, 'id' | 'name' | 'link' | 'capabilities'>;
  /** How often a join in progress is read again. */
  pollMs?: number;
  /** How long to read a join again before leaving it to the Device. */
  pollForMs?: number;
}

const detailsReason = (error: ApiError): string =>
  typeof error.details === 'object' && error.details !== null && 'reason' in error.details && typeof error.details.reason === 'string'
    ? error.details.reason
    : '';

export function createDeviceWifi(deps: DeviceWifiDeps) {
  const { companion, pollMs = 1000, pollForMs = 30_000 } = deps;
  let device = deps.device;
  const store = createStore<DeviceWifiState>()(() => ({ load: 'idle', wifi: null, busy: null, problem: {}, needs: false, error: null }));
  const { getState: state, setState: set } = store;
  const name = () => device.name ?? 'the Device';
  const live = () => device.link.state === 'connected';

  let timer: ReturnType<typeof setTimeout> | null = null;
  let pollUntil = 0;
  let seq = 0;

  function stopPolling() {
    if (timer) clearTimeout(timer);
    timer = null;
  }

  function show(wifi: DeviceWifi) {
    set({ wifi, load: 'ready', needs: false, error: null });
    stopPolling();
    if (wifi.state === 'connecting' && Date.now() < pollUntil) {
      timer = setTimeout(() => {
        timer = null;
        if (live()) void read();
      }, pollMs);
    }
  }

  function failed(thrown: unknown, doing: string) {
    const error = toApiError(thrown);
    if (error.code === 'device.needs_firmware') {
      set({ needs: true, error: null });
      return;
    }
    const usbOnly = error.code === 'device.link' && detailsReason(error).includes('usb-only');
    set({
      error: usbOnly ? `Plug ${name()} in with USB to change its Wi-Fi.` : `Couldn't ${doing} ${name()}'s Wi-Fi. Check that it's plugged in, then retry.`,
    });
  }

  async function read(): Promise<void> {
    const mine = ++seq;
    try {
      const wifi = await companion.devices.wifi_get({ device: device.id });
      if (mine === seq) show(wifi);
    } catch (thrown) {
      if (mine !== seq) return;
      stopPolling();
      if (state().load === 'loading') set({ load: 'failed' });
      failed(thrown, 'read');
    }
  }

  const api = {
    async load(): Promise<void> {
      if (!live() || !offersWifi(device)) return;
      if (state().load !== 'ready') set({ load: 'loading' });
      pollUntil = Date.now() + pollForMs;
      await read();
    },

    /** Stores the network on the Device and follows the join. The password is sent once and kept nowhere. */
    async join(ssid: string, password: string): Promise<boolean> {
      const problem = { ssid: ssidProblem(ssid) ?? undefined, password: passwordProblem(password) ?? undefined };
      set({ problem });
      if (problem.ssid || problem.password || state().busy || !live()) return false;
      const mine = ++seq;
      stopPolling();
      set({ busy: 'joining', error: null });
      try {
        const wifi = await companion.devices.wifi_set({ device: device.id }, null, { ssid, password });
        pollUntil = Date.now() + pollForMs;
        set({ busy: null });
        if (mine === seq) show(wifi);
        return true;
      } catch (thrown) {
        set({ busy: null });
        const error = toApiError(thrown);
        if (error.code === 'request.invalid') {
          const reason = detailsReason(error);
          set({ problem: reason.startsWith('ssid') ? { ssid: 'The Device refused that network name.' } : { password: 'The Device refused that password.' } });
        } else {
          failed(thrown, 'change');
        }
        return false;
      }
    },

    async forget(): Promise<boolean> {
      if (state().busy || !live()) return false;
      const mine = ++seq;
      stopPolling();
      set({ busy: 'forgetting', error: null, problem: {} });
      try {
        const wifi = await companion.devices.wifi_forget({ device: device.id });
        set({ busy: null });
        if (mine === seq) show(wifi);
        return true;
      } catch (thrown) {
        set({ busy: null });
        failed(thrown, 'change');
        return false;
      }
    },

    observe(summary: Pick<DeviceSummary, 'id' | 'name' | 'link' | 'capabilities'>) {
      device = summary;
      if (!live()) stopPolling();
    },

    dispose() {
      stopPolling();
      seq += 1;
    },
  };

  return Object.assign(store, api);
}

export type DeviceWifiStore = ReturnType<typeof createDeviceWifi>;
