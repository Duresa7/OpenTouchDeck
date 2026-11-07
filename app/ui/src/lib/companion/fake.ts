import type { DeviceSummary, FirmwareBuild, Layout, NewBoard, SystemInfo } from './api.ts';
import { CLOSE_SESSION, type EventSocket } from './events.ts';
import { createFakeActions } from './fake/fake-actions.ts';
import { createFakeAssets, type FakeAssetsOptions } from './fake/fake-assets.ts';
import { createFakeBrandfetch, type FakeBrandfetchOptions } from './fake/fake-brandfetch.ts';
import { createFakeConsole } from './fake/fake-console.ts';
import { createFakeControls } from './fake/fake-controls.ts';
import { createFakeDeviceSettings } from './fake/fake-device-settings.ts';
import { createFakeFirmware } from './fake/fake-firmware.ts';
import { createFakeIconPacks, type FakeIconPacksOptions } from './fake/fake-icon-packs.ts';
import { createFakeIntegrations, type FakeIntegrationsOptions } from './fake/fake-integrations.ts';
import { createFakeLayouts } from './fake/fake-layouts.ts';
import { createFakeLive } from './fake/fake-live.ts';
import { createFakeNewBoards } from './fake/fake-new-boards.ts';
import { createFakeSessions, type FakeBrowser } from './fake/fake-sessions.ts';
import { createFakeStartup } from './fake/fake-startup.ts';
import { createFakeWifi, type FakeWifiOptions } from './fake/fake-wifi.ts';
import { createFakeWifiSetup, type FakeWifiSetupOptions } from './fake/fake-wifi-setup.ts';
import type { OpenTouchDeckBridge } from './session.ts';

// An in-memory Companion for tests and the demo mode of `pnpm dev:ui`. It answers
// `system.ping`, `system.get`, `devices.list`, `devices.get` and `layouts.get` the way
// docs/companion-api.md says, checks session tokens, and runs the event stream:
// `hello`, `sync`, then `device.added`, `device.updated`, `device.removed` and `job.updated`. fake-layouts.ts
// answers the Layouts, Jobs and Assets prune routes and sends `job.updated`. fake-sessions.ts,
// fake-startup.ts and fake-console.ts answer the session, start at sign-in and console routes.

export interface FakeCompanionOptions {
  devices?: DeviceSummary[];
  /** The Layout each Device stores at the start, by Device id. */
  layouts?: Record<string, Layout>;
  system?: Partial<SystemInfo>;
  /** How long a fake Job runs. */
  jobMs?: number;
  brightness?: Record<string, number>;
  /** The Devices whose Firmware has the photo frame. */
  photoFrame?: readonly string[];
  /** Wi-Fi setup on Devices whose capabilities have `wifi`: how long a join takes and the networks in range. */
  wifiSetup?: Omit<FakeWifiSetupOptions, 'devices'>;
  bundled?: FirmwareBuild;
  newBoards?: NewBoard[];
  /** Browsers paired before the start. */
  browsers?: readonly FakeBrowser[];
  startAtSignIn?: boolean;
  /** How CLIProxyAPI starts: not set up unless `configured`. */
  integrations?: Omit<FakeIntegrationsOptions, 'publish'>;
  assets?: Omit<FakeAssetsOptions, 'publish' | 'brandfetch'>;
  /** Devices on the network and Pairings made before the start. */
  wifi?: Omit<FakeWifiOptions, 'devices' | 'publish'>;
  iconPacks?: Omit<FakeIconPacksOptions, 'publish'>;
  brandfetch?: FakeBrandfetchOptions;
}

type Frame = Record<string, unknown> & { type: string };

const topicKey = (topic: unknown, device: unknown) => `${String(topic)}/${String(device)}`;

const TOPIC_OF_FRAME: ReadonlyMap<string, string> = new Map([
  ['device.console', 'console'],
  ['device.key-mirror', 'key-mirror'],
]);

class FakeSocket implements EventSocket {
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onclose: ((event: { code: number }) => void) | null = null;
  private seq = 0;
  private closing = false;
  private readonly sockets: Set<FakeSocket>;
  readonly token: string;
  readonly topics = new Set<string>();

  constructor(sockets: Set<FakeSocket>, token: string) {
    this.sockets = sockets;
    this.token = token;
  }

  // Frames arrive on a later task, in order, and frames pushed before a close still arrive.
  push(frame: Frame) {
    if (this.closing) return;
    const topic = TOPIC_OF_FRAME.get(frame.type);
    if (topic !== undefined && !this.topics.has(topicKey(topic, frame.device))) return;
    const data = JSON.stringify({ ...frame, seq: this.seq++ });
    setTimeout(() => this.onmessage?.({ data }), 0);
  }

  send(text: string) {
    let request: unknown;
    try {
      request = JSON.parse(text);
    } catch {
      return;
    }
    if (typeof request !== 'object' || request === null || !('op' in request) || !('topic' in request) || !('device' in request)) return;
    if (request.topic === 'state') return;
    const key = topicKey(request.topic, request.device);
    if (request.op === 'subscribe') this.topics.add(key);
    if (request.op === 'unsubscribe') this.topics.delete(key);
  }

  close(code = 1000) {
    if (this.closing) return;
    this.closing = true;
    this.sockets.delete(this);
    setTimeout(() => {
      this.onmessage = null;
      this.onclose?.({ code });
    }, 0);
  }
}

const json = (status: number, body: unknown) =>
  new Response(status === 204 ? null : JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

const failure = (status: number, code: string, message: string, details: unknown = null) =>
  json(status, { error: { code, message, details } });

export function createFakeCompanion(options: FakeCompanionOptions = {}) {
  const devices = new Map((options.devices ?? []).map((d) => [d.id, structuredClone(d)]));
  const system: SystemInfo = { version: '0.2.0', build: 'fake', api: 'v1', mock: true, data_dir_override: false, platform: 'windows', ...options.system };
  const sockets = new Set<FakeSocket>();
  let minted = 0;
  let requests = 0;
  const controls = createFakeControls({
    devices,
    publish,
    stored: (device, orientation) => layouts.stored(device, orientation),
    settable: ({ integration, stream }) => integrations.list().some((found) => found.id === integration && found.streams.some((s) => s.id === stream && s.settable)),
    delayMs: options.jobMs,
  });
  const layouts = createFakeLayouts({ devices, publish, seed: options.layouts, delayMs: options.jobMs, wrote: controls.wrote });
  const settings = createFakeDeviceSettings({ devices, publish, brightness: options.brightness, photoFrame: options.photoFrame });
  const wifiSetup = createFakeWifiSetup({ devices, ...options.wifiSetup });
  const firmware = createFakeFirmware({ devices, publish, bundled: options.bundled, stepMs: options.jobMs });
  const newBoards = createFakeNewBoards({ devices, publish, bundled: options.bundled ?? { version: '1.5.0', build: 'fake' }, boards: options.newBoards, stepMs: options.jobMs });
  const actions = createFakeActions({ devices, publish, stored: layouts.stored });
  const live = createFakeLive({ devices, stored: layouts.stored });
  const sessions = createFakeSessions({ browsers: options.browsers, onRevoke: signOut });
  const startup = createFakeStartup(options.startAtSignIn);
  const consoles = createFakeConsole({ devices, publish });
  const integrations = createFakeIntegrations({ publish, ...options.integrations });
  const iconPacks = createFakeIconPacks({ publish, ...options.iconPacks });
  const brandfetch = createFakeBrandfetch(options.brandfetch);
  const assets = createFakeAssets({ publish, packHits: iconPacks.hits, ...options.assets, brandfetch });
  const wifi = createFakeWifi({ devices, publish, ...options.wifi });

  function mint() {
    minted += 1;
    return sessions.mint({ kind: 'window', label: 'Window' });
  }

  function publish(frame: Frame) {
    const sent = controls.relink(frame);
    for (const socket of sockets) socket.push(sent);
  }

  function signOut(tokens: ReadonlySet<string>) {
    for (const socket of [...sockets].filter((s) => tokens.has(s.token))) {
      socket.push({ type: 'auth.revoked', data: {} });
      socket.close(CLOSE_SESSION);
    }
  }

  // What Electron's preload would hand the window: a fresh session on every ask.
  // Main fires onSessionChanged only after it mints one on its own, which the fake never does.
  const bridge: OpenTouchDeckBridge = {
    version: 1,
    session: () => Promise.resolve(mint()),
    onSessionChanged: () => {},
  };

  async function fetch(input: string | URL | Request, init?: RequestInit): Promise<Response> {
    requests += 1;
    const request = new Request(input, init);
    const url = new URL(request.url);
    if (!url.pathname.startsWith('/api/v1/')) return failure(404, 'route.not_found', 'No route with that path.');
    const path = url.pathname.slice('/api/v1'.length);
    const get = request.method === 'GET';

    if (path === '/system/ping') return get ? json(200, { api: ['v1'] }) : failure(405, 'route.method_not_allowed', 'Use GET.');
    if (path.startsWith('/test/integrations/')) return (await live.test(request, path)) ?? integrations.test(request, path);
    if (path.startsWith('/test/'))
      return (
        (await firmware.test(request, path)) ??
        wifiSetup.test(request, path) ??
        (await newBoards.test(request, path)) ??
        (await actions.test(request, path)) ??
        (await live.test(request, path)) ??
        settings.record(path, await layouts.test(request, path))
      );

    const auth = request.headers.get('Authorization') ?? '';
    const token = auth.startsWith('Bearer ') ? auth.slice('Bearer '.length) : '';
    if (!token) return failure(401, 'auth.missing', 'No session token.');
    const status = sessions.status(token);
    if (status === 'revoked') return failure(401, 'auth.revoked', 'The session was revoked.');
    if (status === 'unknown') return failure(401, 'auth.invalid', 'The session token is unknown.');

    const routed = sessions.handle(request, path, token) ?? (await startup.handle(request, path)) ?? (await consoles.handle(request, path));
    if (routed) return routed;
    const device = /^\/devices\/([^/]+)$/.exec(path);
    if (path === '/system' || path === '/devices' || device) {
      if (!get) return failure(405, 'route.method_not_allowed', 'Use GET.');
      if (path === '/system') return json(200, system);
      if (path === '/devices') return json(200, [...devices.values()]);
      const found = devices.get(decodeURIComponent(device![1]));
      return found ? json(200, found) : failure(404, 'device.not_found', 'No Device with that id.');
    }
    return (
      (await iconPacks.handle(request, path)) ??
      (await assets.handle(request, path)) ??
      (await brandfetch.handle(request, path)) ??
      (await settings.handle(request, path)) ??
      (await wifiSetup.handle(request, path)) ??
      (await firmware.handle(request, path)) ??
      (await newBoards.handle(request, path)) ??
      (await layouts.handle(request, path)) ??
      actions.handle(request, path) ??
      (await integrations.handle(request, path)) ??
      (await wifi.handle(request, path)) ??
      failure(404, 'route.not_found', 'No route with that path.')
    );
  }

  function openSocket(url: string, protocols: string[]): EventSocket {
    const bearer = protocols.find((p) => p.startsWith('touchdeck.bearer.'));
    const token = bearer?.slice('touchdeck.bearer.'.length) ?? '';
    const socket = new FakeSocket(sockets, token);
    if (!new URL(url).pathname.endsWith('/api/v1/events') || !protocols.includes('touchdeck.v1') || sessions.status(token) !== 'live') {
      socket.close(1006);
      return socket;
    }
    sockets.add(socket);
    socket.push({ type: 'hello', api: 'v1', build: system.build, session: { id: 'fake-session', kind: 'window' } });
    socket.push({
      type: 'sync',
      data: { devices: [...devices.values()], jobs: [...layouts.jobs(), ...firmware.jobs(), ...assets.jobs(), ...iconPacks.jobs(), ...newBoards.jobs()], new_boards: newBoards.boards(), approvals: actions.pending(), integrations: integrations.list() },
    });
    return socket;
  }

  return {
    fetch,
    openSocket,
    bridge,
    layouts,
    controls,
    firmware,
    newBoards,
    actions,
    sessions,
    startup,
    consoles,
    integrations,
    assets,
    wifi,
    iconPacks,
    brandfetch,
    /** Sessions minted so far through the bridge. */
    get minted() {
      return minted;
    },
    /** HTTP requests answered so far. */
    get requests() {
      return requests;
    },
    get openSockets() {
      return sockets.size;
    },
    /** Each open socket's opt-in topics, as `topic/device`. */
    get topics() {
      return [...sockets].map((socket) => [...socket.topics]);
    },
    /** A Device appears: `device.added`. */
    add(device: DeviceSummary) {
      devices.set(device.id, structuredClone(device));
      publish({ type: 'device.added', device: device.id, data: device });
    },
    /** A Device changes: its `rev` goes up and `device.updated` carries the whole object. */
    update(id: string, change: Partial<Omit<DeviceSummary, 'id' | 'rev'>>) {
      const current = devices.get(id);
      if (!current) throw new Error(`no fake Device ${id}`);
      const next = { ...current, ...change, rev: current.rev + 1 };
      devices.set(id, next);
      publish({ type: 'device.updated', device: id, data: next });
      return next;
    },
    /** A `layouts.push` Job puts `layout` on a Device and succeeds: `job.updated`. */
    setLayout(id: string, layout: Layout) {
      layouts.pushed(id, layout);
    },
    /** The Device loses its stored Layout and shows the built-in one, as after a Firmware erase. */
    clearLayout(id: string) {
      const orientation = devices.get(id)?.orientation === 'portrait' ? 'portrait' : 'landscape';
      layouts.forget(id, orientation);
      this.update(id, { layout: { source: 'builtin', name: 'Built-in', orientation, pages: 2, page: 0 } });
    },
    /** A Device is forgotten: `device.removed`. */
    remove(id: string) {
      devices.delete(id);
      publish({ type: 'device.removed', data: { id } });
    },
    /** Reset pairing: every session ends, sockets get `auth.revoked` and close 4401. */
    revokeSessions() {
      sessions.revokeAll();
    },
    /** Drops every socket with `code`, as a restart (1001) or a slow client (4008) would. */
    dropSockets(code: number) {
      for (const socket of [...sockets]) socket.close(code);
    },
    /** Sends a frame as is, such as an event type this client doesn't know. */
    publish,
  };
}

export type FakeCompanion = ReturnType<typeof createFakeCompanion>;

// Explicit fake helpers shared by tests and browser scenarios.
export { createFakeAssets, type FakeSearch } from './fake/fake-assets.ts';
export { createFakeBrandfetch, type FakeBrandfetchOptions } from './fake/fake-brandfetch.ts';
export { createFakeIconPacks, fakeSvg, FAKE_PACKS } from './fake/fake-icon-packs.ts';
export { RECORDED_HITS } from './fake/fake-catalog.data.ts';
export { isAllowedConsoleLine } from './fake/fake-console.ts';
export { hasControls, type Settable } from './fake/fake-controls.ts';
export { type FakePick } from './fake/fake-firmware.ts';
export { fakeGif } from './fake/fake-gif.ts';
export { createFakeIntegrations } from './fake/fake-integrations.ts';
export { createFakeLive } from './fake/fake-live.ts';
export { fakeMedia, fakeVolume } from './fake/fake-media.ts';
export { freshDevice, newBoard } from './fake/fake-new-boards.ts';
export { createFakeSessions } from './fake/fake-sessions.ts';
export { fakeThisPc } from './fake/fake-this-pc.ts';
export { FAKE_NETWORKS } from './fake/fake-wifi-setup.ts';
export { createFakeUpdates, sampleOffer, type FakeUpdatesScript } from './fake/fake-updates.ts';
export { FAKE_WIFI_CODE } from './fake/fake-wifi.ts';
export { LONG_WORD, WORST_DEVICE_NAMES, WORST_LABELS, worstDevices, worstLayout, worstLayouts } from './fake/fake-worst.ts';
