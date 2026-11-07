import type { DeviceSummary, DeviceWifi } from '../api.ts';

// Wi-Fi setup on mock Devices: the network a Device joins, `devices.wifi_*` in docs/companion-api.md. Only a Device
// whose capabilities say `wifi: true` answers; the others need Firmware, and one on Wi-Fi takes it only over USB.
// Pairing over that network is fake-wifi.ts. Every network and password is made up.

/** The networks in range of every fake Device, by name, with their passwords. `''` is an open network. */
export const FAKE_NETWORKS: Readonly<Record<string, string>> = {
  'td-demo-network': 'demo-pass-1234',
  'td-demo-open': '',
};

export interface FakeWifiSetupOptions {
  devices: Map<string, DeviceSummary>;
  networks?: Readonly<Record<string, string>>;
  /** How long a join takes before it connects or fails. */
  joinMs?: number;
}

interface Station {
  ssid: string;
  password: string;
  since: number;
}

const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

const failure = (status: number, code: string, message: string, details: unknown = null) => json(status, { error: { code, message, details } });

const special = (text: string) => [...text].some((c) => c <= '\u001f' || c === '\u007f');
const bytes = (text: string) => new TextEncoder().encode(text).length;
const validSsid = (ssid: string) => ssid.length > 0 && bytes(ssid) <= 32 && !special(ssid);
const validPassword = (password: string) =>
  password === '' || /^[\x20-\x7e]{8,63}$/u.test(password) || /^[0-9a-f]{64}$/iu.test(password);

export function createFakeWifiSetup(options: FakeWifiSetupOptions) {
  const { devices, networks = FAKE_NETWORKS, joinMs = 800 } = options;
  const stations = new Map<string, Station>();

  function view(id: string): DeviceWifi {
    const station = stations.get(id);
    if (!station) return { configured: false, ssid: '', state: 'off', reason: null, ip: null, rssi: null };
    const base = { configured: true, ssid: station.ssid, reason: null, ip: null, rssi: null };
    if (Date.now() - station.since < joinMs) return { ...base, state: 'connecting' };
    const expected = networks[station.ssid];
    if (expected === undefined) return { ...base, state: 'failed', reason: 'not-found' };
    if (expected !== station.password) return { ...base, state: 'failed', reason: 'wrong-password' };
    return { ...base, state: 'connected', ip: '192.0.2.10', rssi: -52 };
  }

  function reachable(id: string): DeviceSummary | Response {
    const device = devices.get(id);
    if (!device) return failure(404, 'device.not_found', 'No Device with that id.');
    if (device.link.state !== 'connected') return failure(409, 'device.not_connected', 'The Device is not connected.', { link: device.link });
    if (device.capabilities?.wifi !== true) return failure(409, 'device.needs_firmware', 'This needs newer Firmware on the Device.', { field: 'wifi', version: null });
    if (device.carrier === 'wifi') return failure(409, 'device.usb_only', 'Change the Device\'s Wi-Fi network over USB.');
    return device;
  }

  async function put(request: Request, id: string): Promise<Response> {
    const body: unknown = await request.json().catch(() => null);
    // A body that doesn't parse gets one fixed reason, so nothing of it comes back.
    const shape = failure(400, 'request.invalid', 'The request body is invalid.', { part: 'body', reason: 'the body is {"ssid": string, "password": string}' });
    if (typeof body !== 'object' || body === null || Object.keys(body).some((key) => key !== 'ssid' && key !== 'password')) return shape;
    const ssid: unknown = Reflect.get(body, 'ssid');
    const password: unknown = Reflect.get(body, 'password');
    if (typeof ssid !== 'string' || typeof password !== 'string') return shape;
    if (!validSsid(ssid)) return failure(400, 'request.invalid', 'The request body is invalid.', { part: 'body', reason: 'ssid is 1 to 32 bytes without control characters' });
    if (!validPassword(password)) {
      return failure(400, 'request.invalid', 'The request body is invalid.', { part: 'body', reason: 'password is empty for an open network, 8 to 63 printable ASCII characters, or 64 hex digits' });
    }
    stations.set(id, { ssid, password, since: Date.now() });
    return json(200, view(id));
  }

  return {
    async handle(request: Request, path: string): Promise<Response | null> {
      const match = /^\/devices\/([^/]+)\/wifi$/.exec(path);
      if (!match) return null;
      const device = reachable(decodeURIComponent(match[1]));
      if (device instanceof Response) return device;
      if (request.method === 'GET') return json(200, view(device.id));
      if (request.method === 'PUT') return put(request, device.id);
      if (request.method === 'DELETE') {
        stations.delete(device.id);
        return json(200, view(device.id));
      }
      return failure(405, 'route.method_not_allowed', 'Use GET, PUT or DELETE.');
    },

    /** `/test/devices/{id}/wifi`: what the Device stored, password included, for a test to check what was sent. */
    test(request: Request, path: string): Response | null {
      const match = /^\/test\/devices\/([^/]+)\/wifi$/.exec(path);
      if (!match) return null;
      if (request.method !== 'GET') return failure(405, 'route.method_not_allowed', 'Use GET.');
      const station = stations.get(decodeURIComponent(match[1]));
      return json(200, station ? { ssid: station.ssid, password: station.password } : null);
    },
  };
}
