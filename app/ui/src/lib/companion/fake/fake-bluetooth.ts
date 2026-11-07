import type { BluetoothBond, DeviceBluetooth, DeviceSummary } from '../api.ts';

// The radio build's Bluetooth keyboard on fake Devices: `devices.bluetooth_*` in docs/companion-api.md. Only a Device
// whose capabilities have `ble_hid` answers; the others need Firmware, and one on Wi-Fi takes it only over USB. The
// tests and the demo play the host's part: `showPasskey` and `hostPairs`. The addresses are documentation addresses
// (RFC 7042).

export interface FakeKeyboardSeed {
  enabled?: boolean;
  bonds?: readonly BluetoothBond[];
}

export interface FakeBluetoothOptions {
  devices: Map<string, DeviceSummary>;
  /** How each Device's keyboard starts, by Device id. Unseeded ones start off with no bonds, as a fresh Device. */
  seed?: Readonly<Record<string, FakeKeyboardSeed>>;
  /** How long the pairing window stays open. */
  windowMs?: number;
  /** How long Bluetooth says `starting` after it is turned on. */
  startMs?: number;
}

interface Keyboard {
  enabled: boolean;
  since: number;
  failed: boolean;
  windowUntil: number | null;
  passkey: boolean;
  bonds: BluetoothBond[];
}

const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

const failure = (status: number, code: string, message: string, details: unknown = null) => json(status, { error: { code, message, details } });

const ADDRESS = /^[0-9a-f]{2}(?::[0-9a-f]{2}){5}$/u;

export function createFakeBluetooth(options: FakeBluetoothOptions) {
  const { devices, windowMs = 120_000, startMs = 0 } = options;
  const keyboards = new Map<string, Keyboard>();
  let startFails = false;

  function keyboard(id: string): Keyboard {
    let found = keyboards.get(id);
    if (!found) {
      const seed = options.seed?.[id];
      found = { enabled: seed?.enabled ?? false, since: 0, failed: false, windowUntil: null, passkey: false, bonds: (seed?.bonds ?? []).map((b) => ({ ...b })) };
      keyboards.set(id, found);
    }
    if (found.windowUntil !== null && Date.now() >= found.windowUntil) {
      found.windowUntil = null;
      found.passkey = false;
    }
    return found;
  }

  function stack(k: Keyboard): DeviceBluetooth['state'] {
    if (!k.enabled) return 'off';
    if (k.failed) return 'failed';
    return Date.now() - k.since < startMs ? 'starting' : 'on';
  }

  function view(id: string, bondsMax: number): DeviceBluetooth {
    const k = keyboard(id);
    const state = stack(k);
    const on = state === 'on';
    const pairing = on && k.windowUntil !== null;
    const waiting = k.bonds.length > 0 && !k.bonds.some((b) => b.connected);
    return {
      enabled: k.enabled,
      state,
      pairing,
      pairing_ms: pairing && k.windowUntil !== null ? Math.max(0, k.windowUntil - Date.now()) : 0,
      passkey: pairing && k.passkey,
      advertising: !on ? 'off' : pairing ? 'pairing' : waiting ? 'fast' : 'off',
      bonds: on ? k.bonds.map((b) => ({ ...b })) : [],
      bonds_max: bondsMax,
      battery: 100,
    };
  }

  function reachable(id: string): DeviceSummary | Response {
    const device = devices.get(id);
    if (!device) return failure(404, 'device.not_found', 'No Device with that id.');
    if (device.link.state !== 'connected') return failure(409, 'device.not_connected', 'The Device is not connected.', { link: device.link });
    if (!device.capabilities?.ble_hid) return failure(409, 'device.needs_firmware', 'This needs newer Firmware on the Device.', { field: 'bluetooth', version: null });
    if (device.carrier === 'wifi') return failure(409, 'device.usb_only', 'The Device takes this only over USB. Plug it in with a USB cable.');
    return device;
  }

  function refused(id: string, max: number, code: 'bluetooth.off' | 'bluetooth.starting' | 'bluetooth.full' | 'bluetooth.radio', message: string) {
    return failure(code === 'bluetooth.radio' ? 502 : 409, code, message, { bluetooth: view(id, max) });
  }

  /** Why the keyboard can't open a window or forget a bond now, or null when it can. */
  function notOn(id: string, max: number): Response | null {
    const state = stack(keyboard(id));
    if (state === 'starting') return refused(id, max, 'bluetooth.starting', 'Bluetooth is still starting on the Device. Try again in a moment.');
    if (state !== 'on') return refused(id, max, 'bluetooth.off', 'Bluetooth is off on the Device. Turn it on first.');
    return null;
  }

  async function set(request: Request, id: string, max: number): Promise<Response> {
    const body: unknown = await request.json().catch(() => null);
    const enabled: unknown = typeof body === 'object' && body !== null && Object.keys(body).length === 1 ? Reflect.get(body, 'enabled') : undefined;
    if (typeof enabled !== 'boolean') {
      return failure(400, 'request.invalid', 'The request body is invalid.', { part: 'body', reason: 'the body is {"enabled": boolean}' });
    }
    const k = keyboard(id);
    if (enabled && !k.enabled) k.since = Date.now();
    k.enabled = enabled;
    k.failed = enabled && startFails;
    if (!enabled || k.failed) {
      k.windowUntil = null;
      k.passkey = false;
    }
    if (k.failed) return refused(id, max, 'bluetooth.radio', "Bluetooth didn't start on the Device. The setting stays on.");
    return json(200, view(id, max));
  }

  function forget(id: string, max: number, address: string | null): Response {
    if (address !== null && !ADDRESS.test(address)) {
      return failure(400, 'request.invalid', 'The request path is invalid.', { part: 'path', reason: 'address is six hex bytes, aa:bb:cc:dd:ee:ff' });
    }
    const blocked = notOn(id, max);
    if (blocked) return blocked;
    const k = keyboard(id);
    const before = k.bonds.length;
    k.bonds = address === null ? [] : k.bonds.filter((b) => b.address !== address);
    return json(200, { forgotten: before - k.bonds.length, bluetooth: view(id, max) });
  }

  return {
    async handle(request: Request, path: string): Promise<Response | null> {
      const match = /^\/devices\/([^/]+)\/bluetooth(\/pairing|\/bonds(?:\/([^/]+))?)?$/u.exec(path);
      if (!match) return null;
      const device = reachable(decodeURIComponent(match[1]));
      if (device instanceof Response) return device;
      const id = device.id;
      const max = device.capabilities?.ble_hid ?? 0;
      const method = request.method;
      const [, , rest, bond] = match;
      if (rest === undefined) {
        if (method === 'GET') return json(200, view(id, max));
        if (method === 'PUT') return set(request, id, max);
        return failure(405, 'route.method_not_allowed', 'Use GET or PUT.');
      }
      if (rest === '/pairing') {
        if (method !== 'POST' && method !== 'DELETE') return failure(405, 'route.method_not_allowed', 'Use POST or DELETE.');
        const blocked = notOn(id, max);
        if (blocked) return blocked;
        const k = keyboard(id);
        if (method === 'DELETE') {
          k.windowUntil = null;
          k.passkey = false;
        } else if (k.bonds.length >= max) {
          return refused(id, max, 'bluetooth.full', 'The Device keeps as many computers and phones as it can. Forget one first.');
        } else {
          k.windowUntil = Date.now() + windowMs;
        }
        return json(200, view(id, max));
      }
      if (method !== 'DELETE') return failure(405, 'route.method_not_allowed', 'Use DELETE.');
      return forget(id, max, bond === undefined ? null : decodeURIComponent(bond).toLowerCase());
    },

    /** A computer or phone starts pairing in the open window, so the Device shows a passkey; `false` takes it down. */
    showPasskey(id: string, shown: boolean) {
      const k = keyboard(id);
      k.passkey = shown && k.windowUntil !== null;
    },

    /** A computer or phone types the passkey and bonds. The new bond closes the window. `false` when no window is open. */
    hostPairs(id: string, address: string): boolean {
      const k = keyboard(id);
      if (k.windowUntil === null || !ADDRESS.test(address)) return false;
      k.bonds = [...k.bonds.filter((b) => b.address !== address).map((b) => ({ ...b, connected: false })), { address, random: false, connected: true }];
      k.windowUntil = null;
      k.passkey = false;
      return true;
    },

    /** Makes the next starts fail, as a controller that won't come up. */
    failStart(fail: boolean) {
      startFails = fail;
    },

    /** What the Device keeps, whether Bluetooth is on or off. */
    state(id: string) {
      const k = keyboard(id);
      return { enabled: k.enabled, bonds: k.bonds.map((b) => ({ ...b })) };
    },
  };
}

export type FakeBluetooth = ReturnType<typeof createFakeBluetooth>;
