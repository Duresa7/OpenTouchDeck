import type { AccountUsage, Capabilities, Delivery, DeviceSummary, IntegrationView, Layout, LinkState, NewBoard, Page, Tile } from '../api.ts';
import type { FakeBrowser } from './fake-sessions.ts';
import { sampleDevice } from '../samples.ts';
import type { UpdateCheck } from '../session.ts';

export const LONG_WORD = 'Supercalifragilisticexpialidocious-StreamingSetupForTheSaturdayEveningShowInHD4K';

const SEEN_MS = Date.UTC(2026, 9, 27, 21, 40);

export const WORST_CAPS: Capabilities = {
  screen: [1024, 600],
  grid: [8, 8],
  pages: 8,
  tiles: 64,
  layout_bytes: 32768,
  layout_schemas: [1, 2],
  storage_free: 0,
  tile_kinds: ['static', 'graph', 'bar', 'now-playing', 'gif'],
  asset_bytes: 262144,
  formats: ['tdi1', 'gif', 'jpeg'],
  image: [64, 64],
  photo_bytes: 0,
  live_hz: 2,
  live_total_hz: 16,
  series: 32,
  text_bytes: 128,
  live_images: 4,
  gif_frames: 60,
  gif_pixels: 983040,
  anim: { image: [128, 128], min_delay_ms: 100, page_tiles: 4, page_pixels: 65536 },
};

/** Device names at the 48-byte limit or past where text usually breaks, in the order the Devices list them. */
export const WORST_DEVICE_NAMES = [
  'Aleksandra Wiśniewska-Kowalczyk’s desk',
  'StreamingDeskLeftOfTheMonitorArmForSaturdayShows',
  'J',
  '配信用メインデスク左側のタッチ盤',
  'مكتب الاستوديو الرئيسي',
  'Phòng thu của Nguyễn Thị Ánh'.normalize('NFD'),
  '👩🏽 Dev desk 🏳️',
  'โต๊ะสตูดิโอชั้น',
  'Bartholomew Fitzgerald-Montgomery III',
  'Desk',
] as const;

const port = (n: number) => `COM${n + 3}`;

const LINKS: ((n: number) => LinkState)[] = [
  (n) => ({ state: 'connected', port: port(n), since_ms: 1, protocol: 2 }),
  (n) => ({ state: 'connected', port: port(n), since_ms: 1, protocol: 2 }),
  () => ({ state: 'lost', reason: 'unplugged', detail: '', at_ms: SEEN_MS }),
  (n) => ({ state: 'connected', port: port(n), since_ms: 1, protocol: 2 }),
  (n) => ({ state: 'connecting', port: port(n) }),
  (n) => ({ state: 'busy', port: port(n), detail: `Access is denied. ${LONG_WORD}` }),
  () => ({ state: 'updating', job: 'job_worst' }),
  () => ({ state: 'offline' }),
  (n) => ({ state: 'unresponsive', port: port(n) }),
  () => ({ state: 'lost', reason: 'timeout', detail: '', at_ms: SEEN_MS }),
];

/** `count` Devices, 0 to 10, with the worst names; `offline` unplugs every one. */
export function worstDevices(count: number, { offline = false } = {}): DeviceSummary[] {
  return WORST_DEVICE_NAMES.slice(0, count).map((name, i) => {
    const id = `mock${String(i + 1).padStart(4, '0')}`;
    const link = offline ? { state: 'lost' as const, reason: 'unplugged' as const, detail: '', at_ms: SEEN_MS } : LINKS[i](i);
    const device = sampleDevice(id, name, link);
    device.capabilities = i === 1 ? null : WORST_CAPS;
    if (device.firmware && i % 3 === 0) device.firmware = { ...device.firmware, version: '1.4.0', update_available: '1.5.0-beta.12+build.20261103' };
    if (i === 3) device.orientation = 'portrait';
    return device;
  });
}

/** Tile labels at the 16-character limit, with every script and shape that breaks text. */
export const WORST_LABELS = [
  'Launch Photoshop',
  'WWWWWWWWWWWWWWWW',
  'J',
  '日本語のボタン名前を入力する',
  'تشغيل الموسيقى',
  'Ünïcödé'.normalize('NFD'),
  '👩‍👩‍👧‍👦 Call mum',
  'Ctrl+Alt+Shift+F',
  'iiiiiiiiiiiiiiii',
  'Wiśniewska',
  '100% CPU',
  '→ Next',
  'Привет мир',
  'โต๊ะสตูดิโอ',
  'OBS: Scene 12',
  'Bartholomew F.',
] as const;

const BROKEN_ICON = 'file:00000000deadbeef';
const BROKEN_ASSET = `sha256:${'0'.repeat(56)}deadbeef`;

const tile = (id: string, label: string, row: number, col: number, rowSpan: number, colSpan: number, action: Tile['action'], extra: Partial<Tile> = {}): Tile => ({
  id, label, row, col, rowSpan, colSpan, icon: null, action, ...extra,
});

const key = (usage: number): Tile['action'] => ({ type: 'key', usage, mods: ['ctrl', 'alt', 'shift'] });
const widget = (source: string, field: string): Tile['action'] => ({ type: 'widget', source, field, account: '' });

function full(): Page {
  const tiles = Array.from({ length: 64 }, (_, i) =>
    tile(`t${i}`, WORST_LABELS[i % WORST_LABELS.length], Math.floor(i / 8), i % 8, 1, 1, key(58 + (i % 12)), i % 5 === 0 ? { icon: BROKEN_ICON } : i % 7 === 0 ? { icon: 'lv:mute' } : {}),
  );
  return { kind: 'grid', id: 'full', title: 'Wiśniewska-Kowalczyk Pro', rows: 8, cols: 8, tiles };
}

const live: Page = {
  kind: 'grid',
  id: 'live',
  title: 'StreamingSceneSwitcherXL',
  rows: 3,
  cols: 4,
  tiles: [
    tile('vol', 'Volume  max', 0, 0, 1, 2, widget('volume', 'level'), { render: 'bar', streams: { value: 'number', muted: 'number' }, input: true }),
    tile('np', 'Now playing', 0, 2, 2, 2, widget('media', 'title'), { render: 'now-playing', streams: { title: 'text', artist: 'text', cover: 'image', state: 'number' } }),
    tile('cpu', '100% CPU', 1, 0, 1, 2, widget('this_pc', 'cpu_pct'), { render: 'graph', streams: { value: 'number' } }),
    tile('gif', 'Broken GIF', 2, 0, 1, 1, { type: 'none' }, { render: 'gif', asset: BROKEN_ASSET }),
    tile('pct', 'Claude 5h usage', 2, 1, 1, 1, widget('cliproxyapi', 'five_hour_pct')),
    tile('plan', 'Plan', 2, 2, 1, 1, widget('cliproxyapi', 'plan')),
    tile('gone', 'Unknown field', 2, 3, 1, 1, widget('cliproxyapi', 'tokens_left')),
  ],
};

const rtl: Page = {
  kind: 'grid',
  id: 'rtl',
  title: 'الصفحة الرئيسية للبث',
  rows: 2,
  cols: 3,
  tiles: [
    tile('ar0', 'تشغيل الموسيقى', 0, 0, 1, 1, { type: 'consumer', usage: 205 }, { icon: 'lv:play_pause' }),
    tile('ar1', 'كتم الصوت', 0, 1, 1, 2, { type: 'consumer', usage: 226 }, { icon: BROKEN_ICON }),
    tile('ar2', 'מצלמה ראשית', 1, 0, 1, 3, key(22)),
  ],
};

const cjk: Page = {
  kind: 'grid',
  id: 'cjk',
  title: '配信用のシーン切り替えページ',
  rows: 2,
  cols: 2,
  tiles: [
    tile('ja0', '日本語のボタン名前を入力する', 0, 0, 1, 1, key(30)),
    tile('ja1', '直播开始', 0, 1, 1, 1, key(31)),
    tile('ko0', '방송 시작하기', 1, 0, 1, 2, key(32)),
  ],
};

const jump: Page = {
  kind: 'grid',
  id: 'jump',
  title: 'J',
  rows: 1,
  cols: 3,
  tiles: [
    tile('j0', 'Back to start', 0, 0, 1, 1, { type: 'page', page: 'full' }),
    tile('j1', 'Launch', 0, 1, 1, 1, { type: 'launch', path: `C:\\Program Files\\${LONG_WORD}\\${LONG_WORD}.exe`, args: [LONG_WORD], cwd: `D:\\${LONG_WORD}` }),
    tile('j2', 'Open', 0, 2, 1, 1, { type: 'open', target: `https://northwind-industries-holdings.example.com/${LONG_WORD}?ref=${LONG_WORD}` }),
  ],
};

/** A full Layout: 8 Pages, 64 Tiles on the first, every label shape, Live tiles, broken icons and pictures. */
export function worstLayout(): Layout {
  return {
    schema: 2,
    name: 'Aleksandra’s Studio Desk',
    orientation: 'landscape',
    pages: [
      full(),
      live,
      rtl,
      cjk,
      { kind: 'dashboard', id: 'usage', title: 'Usage on every account', source: 'cliproxyapi' },
      { kind: 'grid', id: 'empty', title: 'Empty', rows: 3, cols: 4, tiles: [] },
      jump,
      { kind: 'grid', id: 'one', title: 'W', rows: 1, cols: 1, tiles: [tile('w', 'W', 0, 0, 1, 1, { type: 'none' })] },
    ],
  };
}

const tiny: Layout = {
  schema: 1,
  name: 'J',
  orientation: 'landscape',
  pages: [{ kind: 'grid', id: 'j', title: 'J', rows: 1, cols: 1, tiles: [tile('j0', 'J', 0, 0, 1, 1, { type: 'none' })] }],
};

/** What each worst Device stores: the full Layout, nothing at all, or a one-Tile Layout. */
export function worstLayouts(devices: readonly DeviceSummary[]): Record<string, Layout> {
  const layouts: Record<string, Layout> = {};
  devices.forEach((device, i) => {
    if (i % 3 === 1) return;
    const layout = i % 3 === 0 ? worstLayout() : tiny;
    layouts[device.id] = { ...layout, orientation: device.orientation === 'portrait' ? 'portrait' : 'landscape' };
  });
  return layouts;
}

const EMAIL = 'bartholomew.fitzgerald@northwind-industries-holdings.example.com';

/** CLIProxyAPI accounts: unbreakable names, unknown states, and counts and shares far past what anyone expects. */
export function worstAccounts(polled: number): AccountUsage[] {
  const window = (pct: number, delay: number | null) => ({ pct, resets_at_ms: delay === null ? null : polled + delay });
  return [
    { account: EMAIL, provider: 'codex', status: 'active', plan: 'enterprise-unlimited-annual-2026', success: 1_284_567_890, failed: 0, recent_10m: 9_999_999, five_hour: window(999_999_999, 7_800_000), seven_day: window(100, 273_600_000), error: null },
    { account: 'J', provider: 'claude', status: 'quota_exhausted_temporarily', plan: null, success: -3, failed: -1, recent_10m: 0, five_hour: window(-12.5, 0), seven_day: null, error: `The service refused this account's sign-in (HTTP 403). Sign in to it again in CLIProxyAPI. ${LONG_WORD}` },
    { account: LONG_WORD, provider: LONG_WORD, status: 'disabled', plan: LONG_WORD, success: 1, failed: 1, recent_10m: 1, five_hour: null, seven_day: window(0.004, null), error: null },
    { account: '小林さんの仕事用アカウント', provider: 'gemini-cli', status: 'error', plan: 'free', success: 0, failed: 1_000_000, recent_10m: 1, five_hour: null, seven_day: null, error: null },
    { account: 'حساب العمل الرئيسي', provider: 'claude', status: 'active', plan: 'Claude Max', success: 42, failed: 0, recent_10m: 1, five_hour: window(50, 120_000), seven_day: window(99.6, 600_000_000), error: null },
  ];
}

/** Every Device's delivery failed, with the Companion's longest kind of reason. */
export const worstDelivery = (devices: readonly DeviceSummary[]): Delivery[] =>
  devices.map((d, i) => ({ device: d.id, sent: i === 0 ? Number.MAX_SAFE_INTEGER : 0, error: i % 2 ? null : `The Device stopped answering while the Companion sent ${LONG_WORD}.` }));

const SOURCE_NAMES = [
  'Living room temperature from the weather station on the north balcony, in Celsius',
  LONG_WORD,
  'J',
  '東京オフィスのサーバー室の温度',
  'درجة حرارة غرفة الخوادم',
  '👩‍👩‍👧‍👦 family calendar',
  'Kitchen tv 👩‍👩‍👧‍👦',
];

/** The Custom Integration with as many sources as it takes, each named at the 128-byte limit or with no spaces. */
export function worstCustom(enabled = true): IntegrationView {
  const sources = SOURCE_NAMES.map((name, i) => ({ id: `source_${i}_${'x'.repeat(30)}`, name, interval_s: 3600, kind: 'number', input: { kind: 'http', url: `https://northwind-industries-holdings.example.com/${LONG_WORD}`, pointer: '/value', headers: [] } }));
  return {
    config: { enabled, sources },
    secrets: {},
    detail: null,
    summary: {
      revision: 0, id: 'custom', name: 'Custom', icon: 'code', enabled,
      status: { state: 'error', message: `Couldn't read ${LONG_WORD}: the request timed out after 30 s.`, delivery: [] },
      config_schema: [{ name: 'enabled', kind: 'boolean', secret: false }, { name: 'sources', kind: 'json', secret: false }],
      streams: sources.map((s) => ({ id: s.id, kind: 'numeric', unit: null, settable: false })),
      actions: ['refresh'],
      presets: sources.flatMap((s) => [
        { id: `${s.id}_value`, name: `${s.name} value`, action: { type: 'widget', source: 'custom', account: '', field: s.id } },
        { id: `${s.id}_graph`, name: `${s.name} graph`, action: { type: 'widget', source: 'custom', account: '', field: s.id }, render: 'graph' as const },
      ]),
    },
  };
}

export const worstHubError = `connection refused: ${LONG_WORD}`;

/** Ten paired browsers, labelled the way user agents name themselves. */
export const worstBrowsers = (count: number): FakeBrowser[] =>
  Array.from({ length: count }, (_, i) => ({
    label: [
      'Microsoft Edge 142.0.3595.53 on Windows 11 Pro for Workstations',
      LONG_WORD,
      'J',
      'Firefox on 小林さんのノートパソコン',
      'Safari على جهاز iPad الخاص بالمكتب',
    ][i % 5],
    created_ms: Date.UTC(2026, 9, 1 + i, 9, 30),
    last_used_ms: Date.UTC(2026, 9, 28, 20, 41 - i),
  }));

/** An update whose version, notes and links are as long as a release can make them. */
export const worstOffer: UpdateCheck = {
  state: 'available',
  version: '10.12.100-beta.12+build.20261103',
  current: '0.2.0-dev.1234+gabcdef0123456789',
  notes: [
    '## What’s new in this release, with a heading long enough to wrap twice in a 440 pixel dialog',
    `- Fixes ${LONG_WORD}`,
    `- See https://northwind-industries-holdings.example.com/releases/${LONG_WORD}`,
    '- 配信中にデバイスが再接続されると、タイルが正しく表示されるようになりました。',
    '- إصلاح مشكلة في عرض الصفحات عند إعادة الاتصال',
    ...Array.from({ length: 30 }, (_, i) => `- Change ${i + 1}: Volume Tiles follow your finger as you drag.`),
  ].join('\n'),
  date: '2026-11-02T09:00:00Z',
};

export const worstFolder = `D:\\Pictures\\${LONG_WORD}\\${LONG_WORD}`;

/** New boards waiting on ports the way Windows names them past COM9. */
export const worstNewBoards = (): NewBoard[] => [
  { id: 'board_worst_1', port: 'COM127', seen_ms: SEEN_MS, probe: 'other-firmware', product: LONG_WORD, detail: `ESP-ROM:esp32s3-20210327 ${LONG_WORD}` },
];
