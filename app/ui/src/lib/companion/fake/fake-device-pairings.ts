import type { DevicePairing, DevicePairings, DeviceSummary } from '../api.ts';

// The apps a fake radio-build Device keeps Pairings with: `devices.pairings_*` in docs/companion-api.md. Only a Device
// whose capabilities have `wifi` answers, and only over USB. This app's own Pairing comes from the fake Wi-Fi routes, so
// pairing there adds it here and forgetting it here drops the key there. The tests and the demo can lose the answer to
// the next forget, as a cable pulled at the wrong moment.

/** This app's id, as the fake Devices list it. */
export const FAKE_APP_ID = '5e'.repeat(16);
/** The name this app pairs under. */
export const FAKE_APP_NAME = 'Open TouchDeck on Workstation';

const MAX = 4;
const APP = /^[0-9a-f]{32}$/u;

export interface FakeApp {
  app: string;
  name: string;
}

/** What the fake Wi-Fi routes hold for this app. */
export interface FakeHere {
  paired(device: string): boolean;
  /** Drops the key, as forgetting the Pairing here does. */
  forget(device: string): void;
  /** Marks the key as being removed, or not. */
  removing(device: string, on: boolean): void;
}

export interface FakeDevicePairingsOptions {
  devices: Map<string, DeviceSummary>;
  here: FakeHere;
  /** The other apps each Device keeps, oldest first, by Device id. */
  seed?: Readonly<Record<string, readonly FakeApp[]>>;
}

const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

const failure = (status: number, code: string, message: string, details: unknown = null) => json(status, { error: { code, message, details } });

export function createFakeDevicePairings(options: FakeDevicePairingsOptions) {
  const { devices, here } = options;
  const others = new Map<string, FakeApp[]>(Object.entries(options.seed ?? {}).map(([id, apps]) => [id, apps.map((a) => ({ ...a }))]));
  const losing = new Set<string>();
  /** Devices whose forget of this app went unanswered: the next list sends it again. */
  const pending = new Set<string>();
  let storageFails = false;

  const appsOf = (id: string) => others.get(id) ?? [];

  function list(id: string): DevicePairing[] {
    const listed: DevicePairing[] = appsOf(id).map((a) => ({ app: a.app, name: a.name, this_app: false }));
    if (here.paired(id)) listed.push({ app: FAKE_APP_ID, name: FAKE_APP_NAME, this_app: true });
    return listed;
  }

  const view = (id: string): DevicePairings => ({ pairings: list(id), max: MAX, pairing: false, locked_ms: 0 });

  function reachable(id: string): DeviceSummary | Response {
    const device = devices.get(id);
    if (!device) return failure(404, 'device.not_found', 'No Device with that id.');
    if (device.link.state !== 'connected') return failure(409, 'device.not_connected', 'The Device is not connected.', { link: device.link });
    if (device.capabilities?.wifi !== true) return failure(409, 'device.needs_firmware', 'This needs newer Firmware on the Device.', { field: 'pairings', version: null });
    if (device.carrier === 'wifi') return failure(409, 'device.usb_only', 'The Device takes this only over USB. Plug it in with a USB cable.');
    return device;
  }

  function settle(id: string) {
    if (!pending.delete(id)) return;
    here.removing(id, false);
    here.forget(id);
  }

  function forget(id: string, app: string | null): Response {
    if (app !== null && !APP.test(app)) {
      return failure(400, 'request.invalid', 'The request path is invalid.', { part: 'path', reason: 'app is 32 lower-case hex characters' });
    }
    settle(id);
    const own = app === null || app === FAKE_APP_ID;
    const lost = losing.delete(id);
    if (app === null) others.set(id, []);
    else others.set(id, appsOf(id).filter((a) => a.app !== app));
    if (own && here.paired(id)) {
      if (lost) {
        pending.add(id);
        here.removing(id, true);
      } else here.forget(id);
    }
    if (lost) return failure(502, 'device.unknown_outcome', 'The link closed before the Device answered.', { reason: 'the USB link closed' });
    if (storageFails) return failure(502, 'pairings.storage', "The Device couldn't erase the Pairing from its storage. Try again.");
    return json(200, view(id));
  }

  return {
    async handle(request: Request, path: string): Promise<Response | null> {
      const match = /^\/devices\/([^/]+)\/pairings(?:\/([^/]+))?$/u.exec(path);
      if (!match) return null;
      const id = decodeURIComponent(match[1]);
      const app = match[2] === undefined ? null : decodeURIComponent(match[2]);
      const method = request.method;
      if (app === null && method === 'GET') {
        const found = reachable(id);
        if (found instanceof Response) return found;
        settle(id);
        return json(200, view(id));
      }
      if (method !== 'DELETE') return failure(405, 'route.method_not_allowed', app === null ? 'Use GET or DELETE.' : 'Use DELETE.');
      const found = reachable(id);
      if (found instanceof Response) return found;
      return forget(id, app);
    },
    /** The apps the Device lists now. */
    list: (id: string) => list(id),
    /** Another app pairs with the Device, from another computer. */
    add(id: string, app: FakeApp) {
      others.set(id, [...appsOf(id).filter((a) => a.app !== app.app), { ...app }]);
    },
    /** The answer to the next forget on this Device is lost, though the Device acts on it. */
    loseNextForget(id: string) {
      losing.add(id);
    },
    /** Erasing fails on every Device: a forget answers `pairings.storage`. */
    failStorage(fail: boolean) {
      storageFails = fail;
    },
    /** How many apps the Device keeps, for the fake Wi-Fi's `full`. */
    count: (id: string) => list(id).length,
  };
}

export type FakeDevicePairings = ReturnType<typeof createFakeDevicePairings>;
