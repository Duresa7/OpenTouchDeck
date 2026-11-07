import type { Capabilities, DeviceSummary, Layout, LinkState, Page } from './api.ts';

/** A Device that plays GIF Tiles, with the proposed GIF limits from docs/companion-api.md. */
export const GIF_CAPS: Capabilities = {
  screen: [1024, 600],
  grid: [8, 8],
  pages: 8,
  tiles: 64,
  layout_bytes: 32768,
  layout_schemas: [1, 2],
  storage_free: 1_000_000,
  tile_kinds: ['static', 'graph', 'gif'],
  asset_bytes: 262144,
  formats: ['tdi1', 'gif'],
  image: [64, 64],
  photo_bytes: 0,
  live_hz: 2,
  live_total_hz: 16,
  series: 32,
  text_bytes: 0,
  live_images: 0,
  gif_frames: 60,
  gif_pixels: 983040,
  anim: { image: [128, 128], min_delay_ms: 100, page_tiles: 4, page_pixels: 65536 },
};

export function sampleDevice(id: string, name: string | null, link: LinkState = { state: 'offline' }): DeviceSummary {
  return {
    id,
    rev: 1,
    name,
    setup: name === null ? 'needs-setup' : 'ready',
    link,
    carrier: link.state === 'connected' ? 'usb' : null,
    firmware: { version: '1.5.0', protocol: 2, build: 'fake', recovery: false, update_available: null, channel: 'normal' },
    diagnostics: { product: 'touchdeck', running_from: 'ota_0', sinks: ['log', 'link'], current_layout: 'builtin' },
    capabilities: null,
    orientation: 'landscape',
    layout: null,
    control_layout: { state: 'unavailable', reason: 'checking' },
    key_mirror: { enabled: true, reason: null },
    stable_id: true,
  };
}

const connected = (port: string): LinkState => ({ state: 'connected', port, since_ms: 0, protocol: 2 });

export const demoDevices: DeviceSummary[] = [
  sampleDevice('mock0001', 'Desk', connected('COM3')),
  sampleDevice('mock0002', 'Studio', connected('COM4')),
  sampleDevice('mock0003', 'Stream deck by the window', connected('COM5')),
];
if (demoDevices[1].firmware) demoDevices[1].firmware = { ...demoDevices[1].firmware, version: '1.4.0', update_available: '1.5.0' };
// Desk runs the opt-in radio build, so the demo shows the switch back and the onboarding Wi-Fi step.
if (demoDevices[0].firmware) demoDevices[0].firmware = { ...demoDevices[0].firmware, channel: 'radio' };

const TILES: Record<string, string[]> = {
  media: ['Previous', 'Play/Pause', 'Next', 'Mute', 'Volume', 'Music', 'Work', 'Break'],
  work: ['Copy', 'Paste', 'Undo', 'Save', 'Find', 'Home', 'End', 'Escape'],
};

export function demoLayout(order = ['media', 'work']): Layout {
  const pages: Page[] = order.map((id) => ({
    kind: 'grid', id, title: id === 'media' ? 'Media' : 'Work', rows: 2, cols: 4,
    tiles: TILES[id].map((label, i) => ({ id: `${id}${i}`, label, row: Math.floor(i / 4), col: i % 4, rowSpan: 1, colSpan: 1, icon: null, action: { type: 'none' } })),
  }));
  return { schema: 1, name: 'Demo Layout', orientation: 'landscape', pages };
}

export function demoScenario(count: number) {
  return Array.from({ length: count }, (_, i) => ({ ...structuredClone(demoDevices[i % 3]), id: `mock${String(i + 1).padStart(4, '0')}` }));
}
