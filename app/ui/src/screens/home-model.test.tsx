import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { createClient } from '../lib/companion/client.ts';
import { isApiError } from '../lib/companion/errors.ts';
import { createFakeCompanion } from '../lib/companion/fake.ts';
import { demoLayout, sampleDevice } from '../lib/companion/samples.ts';
import { windowSession } from '../lib/companion/session.ts';
import { createUiStore } from '../state/ui.ts';
import { DevicePreview } from './DevicePreview.tsx';
import { lastSeenLayout, readDeviceLayout, seenLayout } from './home-layouts.ts';
import { deviceStatus, deviceWidth, firmwareUpdate, lastSeen, previewOf, statusOf } from './home-model.ts';

describe('Home', () => {
  it.each([
    ['connected', 'Connected', 'ok'], ['connecting', 'Connecting…', 'wait'], ['busy', 'In use by another program', 'bad'],
    ['updating', 'Updating…', 'wait'], ['restarting', 'Restarting…', 'wait'], ['offline', 'Not connected', 'bad'],
    ['lost', 'Not connected', 'bad'], ['disconnected', 'Not connected', 'bad'], ['unresponsive', 'Not connected', 'bad'], ['future', 'Not connected', 'bad'],
  ])('describes %s with words and a status box tone', (state, words, tone) => {
    expect(statusOf(state)).toEqual({ words, live: state === 'connected', tone });
  });
  it.each([
    ['usb', 'connected', 'Connected over USB'], ['wifi', 'connected', 'Connected over Wi-Fi'], [null, 'connected', 'Connected'], ['wifi', 'lost', 'Not connected'],
  ] as const)('names the %s Carrier only while connected', (carrier, state, words) => {
    const device = sampleDevice('one', 'Desk');
    expect(deviceStatus({ carrier, link: { ...device.link, state } as typeof device.link }).words).toBe(words);
  });
  it.each([0, 1, 2, 3, 4, 5])('sizes %i Devices', (count) => {
    expect(deviceWidth(count)).toBe(count === 1 ? 560 : 312);
  });
  it('shows a Firmware badge only when an update is available', () => {
    const device = sampleDevice('one', 'Desk');
    expect(firmwareUpdate(device)).toBe(false);
    device.firmware!.update_available = '1.5.0';
    expect(firmwareUpdate(device)).toBe(true);
    device.firmware = null;
    expect(firmwareUpdate(device)).toBe(false);
  });
  it('renders Page 1 of the Layout, dimmed while offline', () => {
    const preview = previewOf(demoLayout());
    expect(preview?.index).toBe(0);
    const before = renderToStaticMarkup(<DevicePreview preview={preview} live={false} />);
    expect(before).toContain('Media');
    expect(before).toContain('Play/Pause');
    expect(before).toContain('is-off');
    const after = renderToStaticMarkup(<DevicePreview preview={previewOf(demoLayout(['work', 'media']))} live />);
    expect(after).toContain('Work');
    expect(after).toContain('Copy');
    expect(after).not.toContain('Play/Pause');
    expect(after).not.toContain('is-off');
  });
  it('draws the Page the Device shows when its summary names this Layout', () => {
    const layout = demoLayout();
    const shown = { source: 'storage', name: 'Demo Layout', orientation: 'landscape', pages: 2, page: 1 } as const;
    expect(previewOf(layout, shown)?.index).toBe(1);
    const html = renderToStaticMarkup(<DevicePreview preview={previewOf(layout, shown)} live />);
    expect(html).toContain('data-page="work"');
    expect(html).toContain('Copy');
    expect(previewOf(layout, { ...shown, page: null })?.index).toBe(0);
    expect(previewOf(layout, { ...shown, page: 2 })?.index).toBe(0);
    expect(previewOf(layout, { ...shown, name: 'Studio' })?.index).toBe(0);
    expect(previewOf(layout, { ...shown, pages: 3 })?.index).toBe(0);
    expect(previewOf(layout, { ...shown, orientation: 'portrait' })?.index).toBe(0);
    expect(previewOf(layout, { ...shown, source: 'builtin' })?.index).toBe(0);
    expect(previewOf(layout, null)?.index).toBe(0);
  });
  it('says there is no Layout instead of drawing a blank screen', () => {
    const empty = renderToStaticMarkup(<DevicePreview preview={null} live empty />);
    expect(empty).toContain('No Layout yet');
    expect(empty).toContain('Click to make one');
    expect(empty).not.toContain('data-page');
    expect(renderToStaticMarkup(<DevicePreview preview={null} live />)).not.toContain('No Layout yet');
    expect(renderToStaticMarkup(<DevicePreview preview={previewOf(demoLayout())} live empty />)).not.toContain('No Layout yet');
  });
  it('does not invent a Page or a last-seen time', () => {
    expect(previewOf(null)).toBeNull();
    expect(previewOf({ ...demoLayout(), pages: [] })).toBeNull();
    expect(renderToStaticMarkup(<DevicePreview preview={null} live />)).not.toContain('data-page');
    expect(lastSeen(sampleDevice('one', 'Desk'))).toBeNull();
    expect(lastSeen(sampleDevice('one', 'Desk', { state: 'lost', reason: 'unplugged', detail: '', at_ms: 1793137200000 }))).toMatch(/^Last seen/);
  });
  it('starts at Home and carries the chosen Device into Layout', () => {
    const store = createUiStore();
    expect(store.getState().section).toBe('home');
    store.getState().openDevice('two');
    expect(store.getState()).toMatchObject({ section: 'layout', deviceId: 'two' });
    store.getState().openTroubleshooting();
    expect(store.getState()).toMatchObject({ section: 'settings', settingsSection: 'troubleshooting' });
  });
});

describe("a Device's Layout from the fake Companion", () => {
  afterEach(() => vi.unstubAllGlobals());

  it('answers layouts.get as the contract says: connected only, 404 for an empty slot', async () => {
    const connected = { state: 'connected', port: 'COM3', since_ms: 0, protocol: 2 } as const;
    const fake = createFakeCompanion({
      devices: [sampleDevice('a', 'Desk', connected), sampleDevice('b', 'Studio', connected), sampleDevice('c', 'Away')],
      layouts: { a: demoLayout(), c: demoLayout() },
    });
    vi.stubGlobal('fetch', fake.fetch);
    const companion = createClient('http://127.0.0.1:51234', windowSession(fake.bridge));
    const read = (device: string) => companion.layouts.get({ device, orientation: 'landscape' }).catch((e: unknown) => e);
    expect(await read('a')).toMatchObject({ layout: { pages: [{ id: 'media' }, { id: 'work' }] }, revision: expect.stringMatching(/^[0-9a-f]{8}$/) });
    expect(isApiError(await read('b'), 'layout.not_found')).toBe(true);
    expect(isApiError(await read('c'), 'device.not_connected')).toBe(true);
    expect(isApiError(await read('nope'), 'device.not_found')).toBe(true);
    fake.setLayout('b', demoLayout(['work']));
    expect(await read('b')).toMatchObject({ layout: { pages: [{ id: 'work' }] } });
  });

  it("answers layouts.last_seen with this PC's copy, connected or not, and no revision", async () => {
    const connected = { state: 'connected', port: 'COM3', since_ms: 0, protocol: 2 } as const;
    const fake = createFakeCompanion({
      devices: [sampleDevice('a', 'Desk', connected), sampleDevice('c', 'Away')],
      layouts: { a: demoLayout(), c: demoLayout() },
    });
    vi.stubGlobal('fetch', fake.fetch);
    const companion = createClient('http://127.0.0.1:51234', windowSession(fake.bridge));
    const read = (device: string, orientation: 'landscape' | 'portrait' = 'landscape') =>
      companion.layouts.last_seen({ device, orientation }).catch((e: unknown) => e);
    expect(await read('a')).toEqual({ layout: demoLayout() });
    expect(await read('c')).toEqual({ layout: demoLayout() });
    expect(isApiError(await read('c', 'portrait'), 'layout.not_found')).toBe(true);
    expect(isApiError(await read('nope'), 'device.not_found')).toBe(true);
  });

  it("finds an offline Device's Layout in the orientation the PC last saw", async () => {
    const tall = { ...demoLayout(['work']), orientation: 'portrait' } as const;
    const fake = createFakeCompanion({
      devices: [sampleDevice('wide', 'Desk'), sampleDevice('tall', 'Studio'), sampleDevice('none', 'Away')],
      layouts: { wide: demoLayout(), tall },
    });
    vi.stubGlobal('fetch', fake.fetch);
    const companion = createClient('http://127.0.0.1:51234', windowSession(fake.bridge));
    expect(await lastSeenLayout(companion, 'wide', 'landscape')).toEqual(demoLayout());
    expect(await lastSeenLayout(companion, 'tall', 'landscape')).toEqual(tall);
    expect(await lastSeenLayout(companion, 'none', 'landscape')).toBeNull();
  });

  // What the Home hook runs on every link change; the UI tests have no DOM, so they drive it directly.
  describe('the Layout Home keeps for a Device', () => {
    const connected = { state: 'connected', port: 'COM3', since_ms: 0, protocol: 2 } as const;
    const lost = { state: 'lost', reason: 'unplugged', detail: '', at_ms: 1 } as const;

    it('keeps a Layout read while connected after the Device goes offline with no PC copy', async () => {
      const fake = createFakeCompanion({ devices: [sampleDevice('kept', 'Desk', connected)], layouts: { kept: demoLayout() } });
      vi.stubGlobal('fetch', fake.fetch);
      const companion = createClient('http://127.0.0.1:51234', windowSession(fake.bridge));
      const online = readDeviceLayout(companion, 'kept', 'landscape', true);
      await vi.waitFor(() => expect(seenLayout('kept', 'landscape')).toEqual(demoLayout()));
      online();
      fake.layouts.forget('kept', 'landscape');
      fake.update('kept', { link: lost });
      const asked = vi.fn(fake.fetch);
      vi.stubGlobal('fetch', asked);
      readDeviceLayout(companion, 'kept', 'landscape', false);
      // Both orientations come back layout.not_found.
      await vi.waitFor(() => expect(asked).toHaveBeenCalledTimes(2));
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(seenLayout('kept', 'landscape')).toEqual(demoLayout());
    });

    it('reads layouts.get again on reconnect and drops a late offline answer', async () => {
      const fake = createFakeCompanion({ devices: [sampleDevice('back', 'Desk', lost)], layouts: { back: demoLayout() } });
      let release = () => {};
      const held = new Promise<void>((resolve) => { release = resolve; });
      vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = input instanceof Request ? input.url : String(input);
        if (url.includes('/last-seen')) await held;
        return fake.fetch(input, init);
      });
      const companion = createClient('http://127.0.0.1:51234', windowSession(fake.bridge));
      const offline = readDeviceLayout(companion, 'back', 'landscape', false);
      offline();
      fake.update('back', { link: connected });
      readDeviceLayout(companion, 'back', 'landscape', true);
      await vi.waitFor(() => expect(seenLayout('back', 'landscape')).toEqual(demoLayout()));
      fake.setLayout('back', demoLayout(['work']));
      release();
      await new Promise((resolve) => setTimeout(resolve, 20));
      expect(seenLayout('back', 'landscape')).toEqual(demoLayout());
    });
  });
});
