import { createStore } from 'zustand/vanilla';
import type { DeviceBluetooth, DeviceSummary } from '../companion/api.ts';
import type { Companion } from '../companion/client.ts';
import { toApiError } from '../companion/errors.ts';

// The radio build's Bluetooth keyboard (docs/companion-api.md, devices.bluetooth_*): on and off, the pairing window, and
// the Keyboard hosts the Device keeps: the computers and phones it types on. Every request goes over USB only. The store keeps what the Device
// reports; the pairing passkey stays on the Device's screen and never reaches the app.

/** `true` for a Device whose Firmware has the Bluetooth keyboard. */
export function offersBluetooth(device: Pick<DeviceSummary, 'capabilities'>): boolean {
  return (device.capabilities?.ble_hid ?? 0) > 0;
}

/** Milliseconds as `m:ss`, rounded up so the window never reads 0:00 while it is open. */
export function countdown(ms: number): string {
  const seconds = Math.max(0, Math.ceil(ms / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
}

/** One line on how the keyboard is doing. */
export function bluetoothStatusText(bluetooth: DeviceBluetooth, device: string): string {
  // A failed start shows only while the setting is on; turning it off reads off.
  if (!bluetooth.enabled) return `Off. Turn it on to type on a computer or phone with ${device}.`;
  switch (bluetooth.state) {
    case 'off':
      return `Off. Turn it on to type on a computer or phone with ${device}.`;
    case 'starting':
      return 'Starting…';
    case 'failed':
      return `Bluetooth didn't start on ${device}. Restart ${device} to try again.`;
    case 'on':
      if (bluetooth.pairing && bluetooth.passkey) return `${device} shows a 6-digit code. Type it on the computer or phone.`;
      if (bluetooth.pairing) return 'Waiting for a computer or phone…';
      if (bluetooth.bonds.some((b) => b.connected)) return 'On. Connected to a computer or phone.';
      if (bluetooth.bonds.length > 0) return 'On. Waiting for a computer or phone to connect.';
      return `On. Add a computer or phone to type with ${device}.`;
    default: {
      const _exhaustive: never = bluetooth.state;
      return _exhaustive;
    }
  }
}

export type BluetoothBusy = 'switching' | 'opening' | 'closing' | 'forgetting';

export interface BluetoothState {
  load: 'idle' | 'loading' | 'ready' | 'failed';
  bluetooth: DeviceBluetooth | null;
  busy: BluetoothBusy | null;
  /** The address being forgotten, or `all`. */
  forgetting: string | null;
  /** Firmware without the Bluetooth keyboard answered. */
  needs: boolean;
  error: string | null;
  /** A read failed and the next one is due in this many milliseconds, while the panel is open. */
  retryIn: number | null;
}

type Summary = Pick<DeviceSummary, 'id' | 'name' | 'link' | 'capabilities' | 'carrier'>;

export interface BluetoothDeps {
  companion: Pick<Companion, 'devices'>;
  device: Summary;
  /** How often the state is read again while Bluetooth starts or the pairing window is open. */
  pollMs?: number;
  /** The first wait before reading again after a failed read. It doubles up to `retryMaxMs`. */
  retryMs?: number;
  retryMaxMs?: number;
}

const stateIn = (details: unknown): DeviceBluetooth | null =>
  typeof details === 'object' && details !== null && 'bluetooth' in details && typeof details.bluetooth === 'object' && details.bluetooth !== null
    ? (details.bluetooth as DeviceBluetooth)
    : null;

export function createBluetooth(deps: BluetoothDeps) {
  const { companion, pollMs = 1000, retryMs = 1000, retryMaxMs = 16_000 } = deps;
  let device = deps.device;
  const store = createStore<BluetoothState>()(() => ({
    load: 'idle',
    bluetooth: null,
    busy: null,
    forgetting: null,
    needs: false,
    error: null,
    retryIn: null,
  }));
  const { getState: state, setState: set } = store;
  const name = () => device.name ?? 'the Device';
  // The Device takes `ble.*` only over USB, so nothing is sent while it is on Wi-Fi.
  const overUsb = () => device.link.state === 'connected' && device.carrier !== 'wifi';

  let timer: ReturnType<typeof setTimeout> | null = null;
  let watching = false;
  let seq = 0;
  let backoff = retryMs;
  /** The error on show came from a failed read, so the next good read clears it. */
  let readFailed = false;

  function stopPolling() {
    if (timer) clearTimeout(timer);
    timer = null;
    if (state().retryIn !== null) set({ retryIn: null });
  }

  function readIn(ms: number) {
    timer = setTimeout(() => {
      timer = null;
      if (overUsb()) void read();
    }, ms);
  }

  function show(bluetooth: DeviceBluetooth) {
    set({ bluetooth, load: 'ready', needs: false });
    stopPolling();
    if (watching && (bluetooth.state === 'starting' || bluetooth.pairing)) readIn(pollMs);
  }

  /** A read that failed for a passing reason is tried again, waiting longer each time, while the panel is open. */
  function retryLater(code: string) {
    if (!watching || !overUsb() || code === 'device.needs_firmware' || code === 'device.usb_only') return;
    set({ retryIn: backoff });
    readIn(backoff);
    backoff = Math.min(backoff * 2, retryMaxMs);
  }

  function failed(thrown: unknown) {
    const error = toApiError(thrown);
    const shown = stateIn(error.details);
    if (shown) show(shown);
    const max = shown?.bonds_max ?? device.capabilities?.ble_hid ?? 4;
    switch (error.code) {
      case 'device.needs_firmware':
        set({ needs: true, error: null });
        return;
      case 'device.usb_only':
        set({ error: `Plug in ${name()} with a USB cable to change its Bluetooth keyboard.` });
        return;
      case 'bluetooth.off':
        set({ error: 'Turn on the Bluetooth keyboard first.' });
        return;
      case 'bluetooth.starting':
        set({ error: `Bluetooth is still starting on ${name()}. Try again in a moment.` });
        return;
      case 'bluetooth.full':
        set({ error: `${name()} keeps ${max} computers and phones. Forget one to add another.` });
        return;
      case 'bluetooth.radio':
        set({ error: `Bluetooth didn't start on ${name()}. Restart ${name()} to try again.` });
        return;
      case 'bluetooth.storage':
        set({ error: `${name()} couldn't save that. Try again.` });
        return;
      default:
        set({ error: `Couldn't reach ${name()}. Check that it's plugged in, then retry.` });
    }
  }

  async function read(): Promise<void> {
    const mine = ++seq;
    try {
      const bluetooth = await companion.devices.bluetooth_get({ device: device.id });
      if (mine !== seq) return;
      backoff = retryMs;
      if (readFailed) set({ error: null });
      readFailed = false;
      show(bluetooth);
    } catch (thrown) {
      if (mine !== seq) return;
      stopPolling();
      if (state().load === 'loading') set({ load: 'failed' });
      failed(thrown);
      readFailed = true;
      retryLater(toApiError(thrown).code);
    }
  }

  async function run(busy: BluetoothBusy, request: () => Promise<DeviceBluetooth>, forgetting: string | null = null): Promise<boolean> {
    if (state().busy || !overUsb() || !offersBluetooth(device)) return false;
    const mine = ++seq;
    stopPolling();
    set({ busy, forgetting, error: null });
    readFailed = false;
    try {
      const bluetooth = await request();
      set({ busy: null, forgetting: null });
      if (mine === seq) show(bluetooth);
      return true;
    } catch (thrown) {
      set({ busy: null, forgetting: null });
      failed(thrown);
      return false;
    }
  }

  const path = () => ({ device: device.id });

  const api = {
    /** Reads the keyboard and follows it while it starts or the window is open, until `pause`. A failed read is
     * tried again with a growing wait. */
    async load(): Promise<void> {
      watching = true;
      backoff = retryMs;
      if (!overUsb() || !offersBluetooth(device)) return;
      if (state().load !== 'ready') set({ load: 'loading' });
      await read();
    },

    /** Stops following the keyboard, as when the panel closes. */
    pause() {
      watching = false;
      stopPolling();
    },

    setEnabled(enabled: boolean): Promise<boolean> {
      return run('switching', () => companion.devices.bluetooth_set(path(), null, { enabled }));
    },

    openPairing(): Promise<boolean> {
      return run('opening', () => companion.devices.bluetooth_pairing_open(path()));
    },

    closePairing(): Promise<boolean> {
      return run('closing', () => companion.devices.bluetooth_pairing_close(path()));
    },

    forget(address: string): Promise<boolean> {
      return run('forgetting', async () => (await companion.devices.bluetooth_forget({ device: device.id, address })).bluetooth, address);
    },

    forgetAll(): Promise<boolean> {
      return run('forgetting', async () => (await companion.devices.bluetooth_forget_all(path())).bluetooth, 'all');
    },

    observe(summary: Summary) {
      device = summary;
      if (!overUsb()) stopPolling();
    },

    dispose() {
      watching = false;
      stopPolling();
      seq += 1;
    },
  };

  return Object.assign(store, api);
}

export type BluetoothStore = ReturnType<typeof createBluetooth>;
