import type { DeviceSummary, PairingStarted, WifiDevice, WifiPairing } from '../api.ts';

// The `wifi` routes of docs/companion-api.md: discovery, pairing with the code the Device
// shows, and Pairings. Discovery answers `discovered`; a typed address must match one of
// them, with or without the port. A right code links the Device over Wi-Fi when it isn't
// connected, the way the Wi-Fi thread would.

type Frame = Record<string, unknown> & { type: string };

export interface FakeWifiOptions {
  devices: Map<string, DeviceSummary>;
  publish(frame: Frame): void;
  /** Devices on the network: what `wifi.discover` finds and `wifi.pair_start` reaches. */
  discovered?: readonly Omit<WifiDevice, 'paired_here'>[];
  pairings?: readonly WifiPairing[];
  /** The code every Device shows. */
  code?: string;
  /** How many apps a Device keeps now, when its Pairing list says; otherwise discovery's count stands. */
  paired?: (device: string) => number | null;
}

const json = (status: number, body: unknown) =>
  new Response(status === 204 ? null : JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

const failure = (status: number, code: string, message: string, details: unknown = null) => json(status, { error: { code, message, details } });

export const FAKE_WIFI_CODE = '123456';
export const NET_PORT = 47820;
const WINDOW_MS = 120_000;
const ATTEMPTS = 3;
/** The Pairings a Device holds when every slot is taken. */
const FULL = 4;
const KEYCHAIN_MISSING =
  "Windows Credential Manager isn't available, so the Companion can't keep the Pairing key. Sign in to Windows with your own user account and pair again.";

const withPort = (address: string) => (/:\d+$/u.test(address) && !address.endsWith(']') ? address : `${address}:${NET_PORT}`);

async function bodyOf(request: Request): Promise<unknown> {
  try {
    return await request.json();
  } catch {
    return null;
  }
}

const field = (body: unknown, key: string): unknown => (typeof body === 'object' && body !== null && key in body ? (body as Record<string, unknown>)[key] : undefined);

export function createFakeWifi(options: FakeWifiOptions) {
  const { devices, publish, code = FAKE_WIFI_CODE } = options;
  const discovered = (options.discovered ?? []).map((d) => ({ ...d, address: withPort(d.address) }));
  const pairings = new Map((options.pairings ?? []).map((p) => [p.device, structuredClone(p)]));
  const attempts = new Map<string, PairingStarted>();
  let minted = 0;
  let keychain = true;
  let discoveries = 0;

  function link(id: string, change: Partial<DeviceSummary>) {
    const current = devices.get(id);
    if (!current) return;
    const next = { ...current, ...change, rev: current.rev + 1 };
    devices.set(id, next);
    publish({ type: 'device.updated', device: id, data: next });
  }

  function start(address: string): Response {
    const target = withPort(address.trim());
    const found = discovered.find((d) => d.address === target);
    if (!found) return failure(502, 'wifi.unreachable', `Nothing answered at ${target}.`, { reason: 'connection refused' });
    if (pairings.has(found.device)) return failure(409, 'wifi.already_paired', 'This Companion already has a Pairing with the Device.');
    if ((options.paired?.(found.device) ?? found.paired) === FULL)
      return failure(409, 'wifi.full', 'This Device has four paired apps. Remove one on the Device (paired apps screen), or over USB, then pair again.', { paired: FULL });
    minted += 1;
    const started: PairingStarted = { attempt: `attempt_${minted}`, device: found.device, address: found.address, remaining_ms: WINDOW_MS, attempts_left: ATTEMPTS };
    attempts.set(started.attempt, started);
    return json(200, started);
  }

  function submit(attempt: string, body: unknown): Response {
    const digits = field(body, 'code');
    if (typeof digits !== 'string' || !/^\d{6}$/u.test(digits)) return failure(400, 'request.invalid', 'The code is six digits.');
    const started = attempts.get(attempt);
    if (!started) return failure(404, 'wifi.attempt_not_found', 'No started pairing with that id.');
    if (digits !== code) {
      const left = started.attempts_left - 1;
      if (left <= 0) {
        attempts.delete(attempt);
        return failure(429, 'wifi.locked', 'The Device stopped pairing for 30 s after three wrong codes.');
      }
      started.attempts_left = left;
      return failure(422, 'wifi.wrong_code', 'The code is wrong.', { attempts_left: left });
    }
    if (!keychain) return failure(503, 'wifi.keychain', KEYCHAIN_MISSING);
    attempts.delete(attempt);
    const name = discovered.find((d) => d.device === started.device)?.name ?? null;
    const pairing: WifiPairing = { device: started.device, address: started.address, name, confirmed: true, paired_ms: Date.now(), problem: null, removing: false };
    pairings.set(pairing.device, pairing);
    const device = devices.get(pairing.device);
    if (device && device.link.state !== 'connected' && device.link.state !== 'updating') {
      link(pairing.device, { link: { state: 'connected', port: pairing.address, since_ms: Date.now(), protocol: device.firmware?.protocol ?? 2 }, carrier: 'wifi' });
    }
    return json(200, pairing);
  }

  function forget(id: string): Response {
    if (!pairings.delete(id)) return failure(404, 'wifi.not_paired', 'No Pairing with that Device.');
    if (devices.get(id)?.carrier === 'wifi') link(id, { link: { state: 'offline' }, carrier: null });
    return json(204, null);
  }

  async function handle(request: Request, path: string): Promise<Response | null> {
    if (!path.startsWith('/wifi/')) return null;
    const method = request.method;
    if (path === '/wifi/discover') {
      if (method !== 'POST') return failure(405, 'route.method_not_allowed', 'Use POST.');
      discoveries += 1;
      return json(
        200,
        discovered.map((d) => ({ ...d, paired_here: pairings.has(d.device) })),
      );
    }
    if (path === '/wifi/pairings') {
      if (method === 'GET') return json(200, [...pairings.values()]);
      if (method !== 'POST') return failure(405, 'route.method_not_allowed', 'Use GET or POST.');
      const address = field(await bodyOf(request), 'address');
      return typeof address === 'string' && address.trim() ? start(address) : failure(400, 'request.invalid', 'Give an address.');
    }
    const code = /^\/wifi\/attempts\/([^/]+)\/code$/u.exec(path);
    if (code) return method === 'POST' ? submit(decodeURIComponent(code[1]), await bodyOf(request)) : failure(405, 'route.method_not_allowed', 'Use POST.');
    const attempt = /^\/wifi\/attempts\/([^/]+)$/u.exec(path);
    if (attempt) {
      if (method !== 'DELETE') return failure(405, 'route.method_not_allowed', 'Use DELETE.');
      return attempts.delete(decodeURIComponent(attempt[1])) ? json(204, null) : failure(404, 'wifi.attempt_not_found', 'No started pairing with that id.');
    }
    const pairing = /^\/wifi\/pairings\/([^/]+)$/u.exec(path);
    if (pairing) return method === 'DELETE' ? forget(decodeURIComponent(pairing[1])) : failure(405, 'route.method_not_allowed', 'Use DELETE.');
    return failure(404, 'route.not_found', 'No route with that path.');
  }

  return {
    handle,
    /** Whether pairing can store its key: false answers `wifi.keychain`. */
    setKeychain(on: boolean) {
      keychain = on;
    },
    pairings: () => [...pairings.values()].map((p) => ({ ...p })),
    /** This app's side of a Pairing, as the fake Device's Pairing list sees it. */
    here: {
      paired: (device: string) => pairings.has(device),
      forget(device: string) {
        if (pairings.has(device)) forget(device);
      },
      removing(device: string, on: boolean) {
        const found = pairings.get(device);
        if (found) found.removing = on;
      },
    },
    /** Pairings started and not yet finished or cancelled. */
    attempts: () => [...attempts.keys()],
    get discoveries() {
      return discoveries;
    },
  };
}
