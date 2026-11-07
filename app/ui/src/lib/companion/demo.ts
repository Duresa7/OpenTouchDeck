import type { AssetSource, ControlLayout, Delivery, DeviceSummary, ErrorBody, FirmwareChannel, Fit, WifiDevice, WifiPairing } from './api.ts';
import type { Environment } from './connection.ts';
import { createFakeCompanion } from './fake.ts';
import type { FakeSearch } from './fake/fake-assets.ts';
import { worstCaseHits } from './fake/fake-catalog-worst.ts';
import type { FakePick } from './fake/fake-firmware.ts';
import { freshDevice, newBoard, type Answer, type BoardKind } from './fake/fake-new-boards.ts';
import { createFakeUpdates, sampleOffer, type FakeUpdates } from './fake/fake-updates.ts';
import { worstAccounts, worstBrowsers, worstCustom, worstDelivery, worstDevices, worstFolder, worstHubError, worstLayouts, worstNewBoards, worstOffer } from './fake/fake-worst.ts';
import { demoLayout, demoScenario } from './samples.ts';
import { windowSession, type UpdateCheck } from './session.ts';

const checks = {
  none: { state: 'none' },
  available: sampleOffer,
  'no-notes': { ...sampleOffer, notes: null },
  disabled: { state: 'disabled' },
  offline: { state: 'offline' },
  failed: { state: 'failed' },
} satisfies Record<string, UpdateCheck>;

const DEMO_FOLDER = 'D:\\Pictures\\Desk frame';

// Documentation addresses (RFC 5737). The third demo Device is linked over Wi-Fi; the first
// is on the network too, so its Device settings can pair.
const WIFI_ADDRESSES = ['192.0.2.21:47820', null, '192.0.2.23:47820'] as const;

function demoWifi(devices: DeviceSummary[]): { discovered: Omit<WifiDevice, 'paired_here'>[]; pairings: WifiPairing[] } {
  const discovered: Omit<WifiDevice, 'paired_here'>[] = [];
  const pairings: WifiPairing[] = [];
  devices.slice(0, WIFI_ADDRESSES.length).forEach((device, i) => {
    const address = WIFI_ADDRESSES[i];
    if (!address) return;
    discovered.push({ device: device.id, name: device.name ?? 'TouchDeck', address, paired: i === 2 ? 1 : 0 });
    if (i !== 2) return;
    pairings.push({ device: device.id, address, name: device.name, confirmed: true, paired_ms: Date.UTC(2026, 10, 2, 18, 12), problem: null });
    if (device.link.state === 'connected') {
      device.link = { ...device.link, port: address };
      device.carrier = 'wifi';
    }
  });
  return { discovered, pairings };
}

const isCheckName = (name: string | null): name is keyof typeof checks => name !== null && Object.hasOwn(checks, name);

function renderMs(value: string | null): { renderMs?: number } {
  const ms = Number(value);
  return value !== null && Number.isInteger(ms) && ms >= 0 && ms <= 60_000 ? { renderMs: ms } : {};
}

const browsers = [
  { label: 'Edge on Windows', created_ms: Date.UTC(2026, 9, 26, 17, 5), last_used_ms: Date.UTC(2026, 9, 28, 20, 41) },
  { label: 'Firefox on Windows', created_ms: Date.UTC(2026, 9, 27, 9, 30), last_used_ms: Date.UTC(2026, 9, 27, 9, 52) },
];

declare global {
  interface Window {
    touchdeckDemo?: {
      push(id: string, order: string[]): void;
      offline(id: string): void;
      clearLayout(id: string): void;
      device(id: string, change: Partial<Omit<DeviceSummary, 'id' | 'rev'>>): void;
      controls: { set(id: string, layout: ControlLayout): void; hold(id: string): void; release(id: string): void };
      firmware: { hold(id: string, at?: number | 'rebooting'): void; release(id: string): void; cancel(id: string): void; picks(...answers: FakePick[]): void };
      fresh(id: string, firmware?: { version: string; protocol: number; channel?: FirmwareChannel }): void;
      newBoards: {
        plug(board: { id: string; port: string; kind?: BoardKind }): void;
        unplug(id: string): void;
        answer(id: string, answer: Answer): void;
        confirming(id: string): boolean;
        fail(id: string, fail: boolean): void;
        hold(id: string, at?: number | 'done'): void;
        release(id: string): void;
      };
      updates: Pick<FakeUpdates, 'script' | 'finish' | 'calls'>;
      /** The opt-in topics every open event socket subscribed to, as `topic/device`. */
      topics(): string[];
      setHub(up: boolean): void;
      /** Whether pairing over Wi-Fi can store its key in the OS keychain. */
      wifiKeychain(on: boolean): void;
      deliver(entry: Delivery): void;
      /** What the next Choose folder answers: a path, or null for a cancelled dialog. */
      pickFolder(next: string | null): void;
      assets: {
        offline(on: boolean): void;
        holdSearches(on: boolean): void;
        failRender(id: string): void;
        searches(): FakeSearch[];
        created(): AssetSource[];
        fits(): Fit[];
        holdGifs(on: boolean): void;
        failGif(error: ErrorBody): void;
      };
      iconPacks: { failInstall(prefix: string, error: ErrorBody): void; installed(): string[] };
    };
  }
}

const DATA_KEYS = ['data', 'devices', 'offline'] as const;

type DataParams = Partial<Record<(typeof DATA_KEYS)[number], string>>;

const DATA_SETS: { label: string; params: DataParams }[] = [
  { label: 'Demo data', params: {} },
  { label: 'Worst case', params: { data: 'worst' } },
  { label: 'No Devices', params: { data: 'worst', devices: '0' } },
  { label: 'One', params: { data: 'worst', devices: '1' } },
  { label: 'All offline', params: { data: 'worst', offline: 'all' } },
];

function mountDataToggle(target: Window, params: URLSearchParams) {
  if (params.get('toggle') === '0') return;
  const doc = target.document;
  const bar = doc.createElement('div');
  bar.setAttribute('role', 'group');
  bar.setAttribute('aria-label', 'Demo data');
  bar.dataset.demoToggle = '';
  bar.style.cssText = 'position:fixed;left:50%;bottom:8px;translate:-50% 0;z-index:2147483647;display:flex;gap:2px;padding:2px;border-radius:999px;background:var(--fill-active);font:12px system-ui,sans-serif';
  for (const set of DATA_SETS) {
    const chosen = DATA_KEYS.every((name) => (params.get(name) ?? undefined) === set.params[name]);
    const segment = doc.createElement('button');
    segment.type = 'button';
    segment.textContent = set.label;
    segment.setAttribute('aria-pressed', String(chosen));
    segment.style.cssText = `border:0;border-radius:999px;padding:4px 10px;cursor:pointer;color:var(--ink);background:${chosen ? 'var(--surface-raised)' : 'transparent'}`;
    segment.addEventListener('click', () => {
      const url = new URL(target.location.href);
      for (const name of DATA_KEYS) url.searchParams.delete(name);
      for (const name of DATA_KEYS) {
        const value = set.params[name];
        if (value !== undefined) url.searchParams.set(name, value);
      }
      target.location.assign(url);
    });
    bar.append(segment);
  }
  doc.body.append(bar);
}

function deviceCount(params: URLSearchParams, worst: boolean) {
  const max = worst ? 10 : 5;
  const fallback = worst ? 10 : 3;
  const requested = Number(params.get('devices') ?? fallback);
  return Number.isInteger(requested) && requested >= 0 && requested <= max ? requested : fallback;
}

export function demoEnvironment(target: Window): Environment {
  const params = new URL(target.location.href).searchParams;
  const worst = params.get('data') === 'worst';
  const offline = params.get('offline') === 'all';
  const count = deviceCount(params, worst);
  const devices = worst ? worstDevices(count, { offline }) : demoScenario(count);
  if (offline && !worst) for (const device of devices) device.link = { state: 'lost', reason: 'unplugged', detail: '', at_ms: Date.UTC(2026, 9, 27, 21, 40) };
  const fake = createFakeCompanion({
    devices,
    layouts: worst ? worstLayouts(devices) : Object.fromEntries(devices.map((d) => [d.id, demoLayout()])),
    // `?platform=linux` shows Linux wording, such as the keychain's name.
    system: { build: 'demo', platform: params.get('platform') === 'linux' ? 'linux' : 'windows' },
    brightness: Object.fromEntries(devices.filter((_, i) => i % 3 !== 2).map((d) => [d.id, 80])),
    photoFrame: devices.filter((_, i) => i % 3 !== 2).map((d) => d.id),
    browsers: params.get('browsers') === '0' ? [] : worst ? worstBrowsers(10) : browsers,
    newBoards: worst ? worstNewBoards() : undefined,
    integrations: {
      configured: params.get('integrations') !== 'new',
      delivery: worst ? worstDelivery(devices) : devices.map((d) => ({ device: d.id, sent: 42, error: null })),
      ...(worst ? { accounts: worstAccounts, hubMessage: worstHubError, extra: { custom: worstCustom } } : {}),
    },
    assets: { ...(params.get('icons') === 'worst' ? { hits: worstCaseHits() } : {}), ...renderMs(params.get('render-ms')) },
    wifi: worst ? undefined : demoWifi(devices),
    // `?packs=logos,mdi` starts with those Icon packs installed.
    iconPacks: { installed: params.get('packs')?.split(',') ?? [] },
    // `?brandfetch=on` starts with fake keys set. Search still goes to Brandfetch, which e2e
    // specs intercept; nobody's real keys are involved. `?keychain=off` acts like Linux with no
    // Secret Service running.
    brandfetch: {
      ...(params.get('brandfetch') === 'on' ? { clientId: 'demo-client-id', keySet: true, logos: { total: 3, needRefresh: 1, expired: [] } } : {}),
      ...(params.get('keychain') === 'off' ? { keychain: false } : {}),
    },
  });
  const update = params.get('update');
  const offer = worst && update === 'available' ? worstOffer : isCheckName(update) ? checks[update] : checks.none;
  const updates = createFakeUpdates({ check: offer });
  mountDataToggle(target, params);
  let picked: string | null = worst ? worstFolder : DEMO_FOLDER;
  const inBrowser = params.get('shell') === 'browser';
  const pickFolder = inBrowser ? undefined : async () => picked;
  target.touchdeckDemo = {
    push: (id, order) => fake.setLayout(id, demoLayout(order)),
    offline: (id) => fake.update(id, { link: { state: 'lost', reason: 'unplugged', detail: '', at_ms: Date.UTC(2026, 9, 27, 21, 40) } }),
    clearLayout: (id) => fake.clearLayout(id),
    device: (id, change) => fake.update(id, change),
    controls: { set: fake.controls.set, hold: fake.controls.hold, release: fake.controls.release },
    firmware: { hold: fake.firmware.hold, release: fake.firmware.release, cancel: fake.firmware.cancel, picks: fake.firmware.picks },
    fresh: (id, firmware = { version: '1.5.0', protocol: 2 }) => fake.add(freshDevice({ id, port: 'COM9', firmware })),
    newBoards: {
      plug: (board) => fake.newBoards.plug(newBoard(board)),
      unplug: fake.newBoards.unplug,
      answer: fake.newBoards.answer,
      confirming: fake.newBoards.confirming,
      fail: (id, fail) => fake.newBoards.outcome(id, fail ? 'fail' : 'succeed'),
      hold: fake.newBoards.hold,
      release: fake.newBoards.release,
    },
    updates: { script: updates.script, finish: updates.finish, calls: updates.calls },
    topics: () => fake.topics.flat(),
    setHub: (up) => fake.integrations.setHub(up),
    wifiKeychain: (on) => fake.wifi.setKeychain(on),
    deliver: (entry) => fake.integrations.deliver(entry),
    pickFolder: (next) => {
      picked = next;
    },
    assets: {
      offline: (on) => fake.assets.setOffline(on),
      holdSearches: (on) => fake.assets.holdSearches(on),
      failRender: (id) => fake.assets.failRender(id),
      searches: () => fake.assets.searches.map((s) => ({ ...s })),
      created: () => structuredClone([...fake.assets.created]),
      fits: () => [...fake.assets.fits],
      holdGifs: (on) => fake.assets.holdGifs(on),
      failGif: (error) => fake.assets.failGif(error),
    },
    iconPacks: { failInstall: (prefix, error) => fake.iconPacks.failInstall(prefix, error), installed: () => fake.iconPacks.installed() },
  };
  const network = target.fetch.bind(target);
  target.fetch = (input, init) => {
    const url = new URL(input instanceof Request ? input.url : String(input), target.location.href);
    return url.origin === target.location.origin && url.pathname.startsWith('/api/') ? fake.fetch(input, init) : network(input, init);
  };
  return { base: target.location.origin, session: windowSession(fake.bridge), open: fake.openSocket, updates: updates.bridge, pickFolder };
}
