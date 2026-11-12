import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import type { Capabilities, ControlLayout } from '../src/lib/companion/api.ts';
import type { GridPage, Layout, Tile } from '../src/lib/layout/model.ts';
import { consoleErrors, sideScroll, spills, underRail } from './break-helpers.ts';
import { errorCard, errorDetail, layoutOf, waitEvent } from './layout-helpers.ts';

const shots = process.env.TD_SHOTS;

const NO_COPY: ControlLayout = { state: 'no-pushed-copy', orientation: 'landscape', has_controls: true };
const READ_FAILED: ControlLayout = { state: 'unavailable', reason: 'read-failed' };
const CHECKING: ControlLayout = { state: 'unavailable', reason: 'checking' };
const NO_COPY_TEXT = "These Tiles show live values, but their controls aren't set up on this PC.";
const UNREAD_TEXT = "Couldn't check Desk's controls. Unplug it and plug it back in.";
const DIFFERS: ControlLayout = { state: 'differs', orientation: 'landscape', has_controls: true };

const widget = (source: string, field: string, account = '') => ({ type: 'widget' as const, source, field, account });
const at = (id: string, label: string, row: number, col: number, colSpan = 1) => ({ id, label, row, col, rowSpan: 1, colSpan });

const volume: Tile = { ...at('volume', 'Volume', 0, 0, 2), icon: null, render: 'bar', streams: { value: 'number', muted: 'number' }, input: true, action: widget('volume', 'level') };
const playing: Tile = { ...at('playing', 'Now playing', 0, 2, 2), icon: null, render: 'now-playing', streams: { title: 'text', artist: 'text', cover: 'image', state: 'number' }, action: widget('media', 'title') };
const forged: Tile = { ...at('forged', 'Volume', 0, 0, 2), icon: null, render: 'bar', streams: { value: 'number' }, input: true, action: widget('forged', 'level', 'other PC') };
const play: Tile = { ...at('play', 'Play', 1, 0), icon: null, action: { type: 'consumer', usage: 205 } };
const gif: Tile = { ...at('gif', 'GIF', 1, 1), icon: null, render: 'gif', asset: `sha256:${'1'.repeat(64)}`, action: { type: 'none' } };

const MEDIA: GridPage = { kind: 'grid', id: 'media', title: 'Media', rows: 2, cols: 4, tiles: [volume, playing, play] };
const MORE: GridPage = { kind: 'grid', id: 'more', title: 'More', rows: 2, cols: 4, tiles: [forged] };

const RECOVERED: Layout = { schema: 2, name: 'Upgraded', orientation: 'landscape', pages: [MEDIA, MORE] };
const REFUSED: Layout = { ...RECOVERED, pages: [{ ...MEDIA, tiles: [...MEDIA.tiles, gif] }, MORE] };

const LIVE_CAPS: Capabilities = {
  screen: [1024, 600],
  grid: [8, 8],
  pages: 8,
  tiles: 64,
  layout_bytes: 32768,
  layout_schemas: [1, 2],
  storage_free: 1_000_000,
  tile_kinds: ['static', 'graph', 'bar', 'now-playing'],
  asset_bytes: 12300,
  formats: ['tdi1', 'jpeg'],
  image: [256, 256],
  photo_bytes: 0,
  live_hz: 2,
  live_total_hz: 16,
  series: 32,
  text_bytes: 128,
  live_images: 4,
};

const card = (page: Page, name: string) => page.locator(`.device-figure:has(.device-hit[aria-label="Edit ${name}"])`);
const notice = (page: Page) => page.getByTestId('controls-notice');

async function onDevice(page: Page, layout: Layout, controls: ControlLayout) {
  await page.evaluate(async (text) => {
    await fetch('/api/v1/test/devices/mock0001/layouts/landscape', { method: 'PUT', body: text });
  }, JSON.stringify(layout));
  await page.evaluate((capabilities) => window.touchdeckDemo?.device('mock0001', { capabilities }), LIVE_CAPS);
  await page.evaluate((status) => window.touchdeckDemo?.controls.set('mock0001', status), controls);
}

async function sentTiles(page: Page): Promise<string[]> {
  const record: unknown = JSON.parse(await page.evaluate(async () => (await fetch('/api/v1/test/devices/mock0001/record')).text()));
  const layouts = typeof record === 'object' && record !== null && 'layouts' in record ? record.layouts : null;
  const landscape = typeof layouts === 'object' && layouts !== null && 'landscape' in layouts ? layouts.landscape : null;
  return layoutOf(JSON.stringify(landscape)).pages.flatMap((p) => (p.kind === 'grid' ? p.tiles.map((t) => t.id) : []));
}

async function home(page: Page) {
  await page.goto('/?toggle=0');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}

async function noSpill(page: Page, name: string) {
  await page.waitForFunction(() => document.getAnimations().every((animation) => animation.playState !== 'running' || animation.effect?.getTiming().iterations === Infinity));
  const rail = await underRail(page, '[data-testid=page-entry], [data-testid=page-add], [data-testid=status]');
  expect({ scroll: await sideScroll(page), spills: await spills(page, ['.preset-name']), rail }, name).toEqual({ scroll: 0, spills: [], rail: [] });
}

async function shot(page: Page, name: string) {
  if (!shots) return;
  mkdirSync(shots, { recursive: true });
  await page.screenshot({ path: join(shots, `${name}.png`) });
}

test('a Device that needs a Send: Home notice, Review Layout, Send, Checking controls and cleared', async ({ page }) => {
  const errors = consoleErrors(page);
  await home(page);
  await onDevice(page, RECOVERED, NO_COPY);

  const desk = card(page, 'Desk');
  await expect(desk.getByTestId('device-status')).toHaveText(/: Connected/);
  await expect(desk.getByTestId('controls-notice')).toHaveAttribute('data-state', 'no-pushed-copy');
  await expect(desk.getByTestId('controls-notice')).toContainText('Controls need Send to Device');
  await expect(card(page, 'Studio').getByTestId('controls-notice')).toHaveCount(0);

  await desk.getByRole('button', { name: 'Review Layout for Desk' }).click();
  await expect(page.getByTestId('editor')).toHaveAttribute('data-device-id', 'mock0001');
  await waitEvent(page, 'pulled');
  await expect(notice(page)).toHaveAttribute('data-state', 'no-pushed-copy');
  await expect(notice(page).getByRole('heading')).toHaveText('Controls need Send to Device');
  await expect(page.getByTestId('controls-notice-body')).toHaveText(NO_COPY_TEXT);
  await expect(notice(page)).toContainText('Sending replaces the Layout on Desk and lets its Tiles control the Integrations shown here.');

  const rows = page.getByTestId('control-binding');
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0)).toContainText('Volume');
  await expect(rows.nth(0).getByTestId('control-integration')).toHaveText('Volume');
  await expect(rows.nth(0).getByTestId('control-account')).toHaveText('Default account');
  await expect(rows.nth(0).getByTestId('control-parts')).toHaveText('Drag to change · Mute');
  await expect(rows.nth(1).getByTestId('control-integration')).toHaveText('Media');
  await expect(rows.nth(1).getByTestId('control-parts')).toHaveText('Previous track · Play or pause · Next track');
  await expect(rows.nth(2)).toHaveAttribute('data-page', 'more');
  await expect(rows.nth(2).getByTestId('control-integration')).toHaveText('Unknown Integration “forged”');
  await expect(rows.nth(2).getByTestId('control-account')).toHaveText('Unknown account “other PC”');

  await rows.nth(2).click();
  await expect(page.getByTestId('tile-title')).toHaveText('Volume');
  await page.getByTestId('tile-clear').click();
  await expect(rows).toHaveCount(2);

  await page.evaluate(() => window.touchdeckDemo?.controls.hold('mock0001'));
  await page.getByTestId('push').click();
  await waitEvent(page, 'pushed');
  await expect(notice(page)).toHaveAttribute('data-state', 'checking');
  await expect(notice(page)).toHaveText('Checking controls…');
  await expect(page.getByTestId('control-review')).toBeVisible();
  expect(await sentTiles(page)).toEqual(['volume', 'playing', 'play']);

  await page.evaluate(() => window.touchdeckDemo?.controls.release('mock0001'));
  await expect(notice(page)).toHaveCount(0);
  await expect(page.getByTestId('control-review')).toHaveCount(0);
  await page.getByRole('button', { name: 'Home', exact: true }).click();
  await expect(card(page, 'Desk').getByTestId('controls-notice')).toHaveCount(0);
  expect(errors).toEqual([]);
});

test('Review Layout goes through the load flow and keeps an unsent draft until the user replaces it', async ({ page }) => {
  await home(page);
  await onDevice(page, RECOVERED, NO_COPY);
  await card(page, 'Desk').locator('.device-hit').click();
  await page.getByTestId('start-new').click();
  await expect(page.getByTestId('editor-head')).toHaveAttribute('data-save-state', 'unsent');
  await page.getByTestId('controls-review').click();
  await expect(page.getByTestId('replace-dialog')).toBeVisible();
  await page.getByTestId('keep-mine').click();
  await expect(page.getByTestId('page-entry')).toHaveCount(1);
  await expect(page.getByTestId('control-review')).toContainText('No Tile on this Layout controls an Integration.');
  await expect(notice(page)).toHaveAttribute('data-state', 'no-pushed-copy');

  await page.getByRole('button', { name: 'Home', exact: true }).click();
  await card(page, 'Desk').getByTestId('controls-review').click();
  await page.getByTestId('replace').click();
  await expect(page.getByTestId('page-entry')).toHaveCount(2);
  await expect(page.getByTestId('control-binding')).toHaveCount(3);
});

test('a Send that fails keeps the guidance and the details', async ({ page }) => {
  await home(page);
  await onDevice(page, REFUSED, NO_COPY);
  await card(page, 'Desk').getByTestId('controls-review').click();
  await waitEvent(page, 'pulled');
  await page.getByTestId('push').click();
  await waitEvent(page, 'push-failed');
  const failed = (await page.getByTestId('status').locator('.editor-status-text').textContent()) ?? '';
  await expect(errorCard(page, failed)).toBeVisible();
  await expect(page.getByTestId('push'), 'a failed send never says Saved').toHaveAttribute('data-state', 'send');
  await expect(notice(page)).toHaveAttribute('data-state', 'no-pushed-copy');
  await expect(page.getByTestId('controls-notice-body')).toHaveText(NO_COPY_TEXT);
  await expect(page.getByTestId('control-binding')).toHaveCount(3);
  await page.getByRole('button', { name: 'Home', exact: true }).click();
  await expect(card(page, 'Desk').getByTestId('controls-notice')).toHaveAttribute('data-state', 'no-pushed-copy');
  await expect(await errorDetail(page, failed)).toContainText('asset.unsupported');
});

test('a check that ends unread says so on Home and in the editor, and Home announces it', async ({ page }) => {
  await home(page);
  await onDevice(page, RECOVERED, NO_COPY);
  await card(page, 'Desk').getByTestId('controls-review').click();
  await waitEvent(page, 'pulled');
  await page.evaluate(() => window.touchdeckDemo?.controls.hold('mock0001'));
  await page.getByTestId('push').click();
  await waitEvent(page, 'pushed');
  await expect(notice(page)).toHaveAttribute('data-state', 'checking');
  await page.evaluate((status) => window.touchdeckDemo?.controls.set('mock0001', status), READ_FAILED);
  await expect(notice(page)).toHaveAttribute('data-state', 'unchecked');
  await expect(notice(page)).toHaveAttribute('role', 'status');
  await expect(notice(page)).toHaveText(UNREAD_TEXT);
  await expect(page.getByTestId('status')).toHaveText('Sent to Desk');

  await page.getByRole('button', { name: 'Home', exact: true }).click();
  const desk = card(page, 'Desk');
  await expect(desk.getByRole('status')).toHaveText(UNREAD_TEXT);
  await expect(desk.getByTestId('controls-review')).toHaveCount(0);
  await page.evaluate((status) => window.touchdeckDemo?.controls.set('mock0001', status), CHECKING);
  await expect(desk.getByRole('status')).toHaveText('Checking controls…');
  await page.evaluate(() => window.touchdeckDemo?.controls.release('mock0001'));
  await expect(desk.getByTestId('controls-notice')).toHaveCount(0);
});

test('a Layout that differs from the one this PC sent says so', async ({ page }) => {
  await home(page);
  await onDevice(page, RECOVERED, DIFFERS);
  await expect(card(page, 'Desk').getByTestId('controls-notice')).toHaveAttribute('data-state', 'differs');
  await card(page, 'Desk').getByTestId('controls-review').click();
  await waitEvent(page, 'pulled');
  await expect(page.getByTestId('controls-notice-body')).toHaveText('The Layout on Desk differs from the one this PC sent.');
});

test('the editor on the other orientation names the one that needs review and switches to it', async ({ page }) => {
  await home(page);
  await onDevice(page, RECOVERED, NO_COPY);
  await card(page, 'Desk').getByTestId('controls-review').click();
  await waitEvent(page, 'pulled');
  await page.getByTestId('orientation-portrait').click();
  await expect(notice(page)).toHaveAttribute('data-state', 'other-orientation');
  await expect(page.getByTestId('controls-notice-body')).toHaveText('Desk shows its landscape Layout. Switch to Landscape to review it.');
  await expect(page.getByTestId('control-review')).toHaveCount(0);
  await page.getByTestId('controls-switch').click();
  await expect(page.getByTestId('orientation-landscape')).toHaveAttribute('aria-pressed', 'true');
  await expect(notice(page)).toHaveAttribute('data-state', 'no-pushed-copy');
  await expect(page.getByTestId('control-binding')).toHaveCount(3);
});

test('an offline Device and a matching or control-free Layout show nothing new', async ({ page }) => {
  await home(page);
  await onDevice(page, RECOVERED, NO_COPY);
  await expect(card(page, 'Desk').getByTestId('controls-notice')).toHaveCount(1);
  await page.evaluate(() => window.touchdeckDemo?.offline('mock0001'));
  await expect(card(page, 'Desk').locator('.device-meta .status')).toHaveText('Not connected');
  await expect(card(page, 'Desk').getByTestId('controls-notice')).toHaveCount(0);
  await card(page, 'Desk').locator('.device-hit').click();
  await expect(page.getByTestId('disconnected')).toBeVisible();
  await expect(notice(page)).toHaveCount(0);

  await home(page);
  for (const status of [
    { state: 'matches', orientation: 'landscape', has_controls: true },
    { state: 'no-pushed-copy', orientation: 'landscape', has_controls: false },
    { state: 'unavailable', reason: 'read-failed' },
  ] satisfies ControlLayout[]) {
    await page.evaluate((next) => window.touchdeckDemo?.controls.set('mock0001', next), status);
    await expect(card(page, 'Desk').getByTestId('device-status')).toHaveText(/: Connected/);
    await expect(page.getByTestId('controls-notice')).toHaveCount(0);
  }
});

for (const theme of ['light', 'dark'] as const) {
  for (const viewport of [{ width: 1100, height: 760 }, { width: 800, height: 600 }]) {
    const size = `${viewport.width}x${viewport.height}`;
    test.describe(`${theme} at ${size}`, () => {
      test.use({ colorScheme: theme, viewport });

      test('the notice, the review list and Checking controls fit without spilling', async ({ page }) => {
        await home(page);
        await onDevice(page, RECOVERED, NO_COPY);
        await expect(card(page, 'Desk').getByTestId('controls-notice')).toBeVisible();
        await noSpill(page, `home-${size}-${theme}`);
        await shot(page, `home-${size}-${theme}`);

        await card(page, 'Desk').getByTestId('controls-review').click();
        await waitEvent(page, 'pulled');
        await expect(page.getByTestId('control-binding')).toHaveCount(3);
        await noSpill(page, `editor-${size}-${theme}`);
        await shot(page, `editor-${size}-${theme}`);

        await page.getByTestId('orientation-portrait').click();
        await expect(notice(page)).toHaveAttribute('data-state', 'other-orientation');
        await noSpill(page, `other-orientation-${size}-${theme}`);
        await shot(page, `other-orientation-${size}-${theme}`);
        await page.getByTestId('controls-switch').click();

        await page.evaluate(() => window.touchdeckDemo?.controls.hold('mock0001'));
        await page.getByTestId('push').click();
        await waitEvent(page, 'pushed');
        await expect(notice(page)).toHaveAttribute('data-state', 'checking');
        await noSpill(page, `checking-${size}-${theme}`);
        await shot(page, `checking-${size}-${theme}`);
        await page.evaluate((status) => window.touchdeckDemo?.controls.set('mock0001', status), READ_FAILED);
        await expect(notice(page)).toHaveAttribute('data-state', 'unchecked');
        await noSpill(page, `unchecked-${size}-${theme}`);
        await shot(page, `unchecked-${size}-${theme}`);
        await page.getByRole('button', { name: 'Home', exact: true }).click();
        await expect(card(page, 'Desk').getByRole('status')).toHaveText(UNREAD_TEXT);
        await noSpill(page, `home-unchecked-${size}-${theme}`);
        await shot(page, `home-unchecked-${size}-${theme}`);
        await page.evaluate(() => window.touchdeckDemo?.controls.release('mock0001'));
        await expect(notice(page)).toHaveCount(0);
      });

      test('a failed Send keeps the notice, the Pages column and the Details clear of the nav', async ({ page }) => {
        await home(page);
        await onDevice(page, REFUSED, NO_COPY);
        await card(page, 'Desk').getByTestId('controls-review').click();
        await waitEvent(page, 'pulled');
        await page.getByTestId('push').click();
        await waitEvent(page, 'push-failed');
        await expect(notice(page)).toHaveAttribute('data-state', 'no-pushed-copy');
        await noSpill(page, `failed-${size}-${theme}`);
        await shot(page, `failed-${size}-${theme}`);

        const failed = (await page.getByTestId('status').locator('.editor-status-text').textContent()) ?? '';
        await expect(await errorDetail(page, failed)).toBeVisible();
        await noSpill(page, `failed-details-${size}-${theme}`);
        await shot(page, `failed-details-${size}-${theme}`);
      });
    });
  }
}
