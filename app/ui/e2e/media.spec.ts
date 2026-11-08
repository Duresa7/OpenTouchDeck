import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import type { Capabilities } from '../src/lib/companion/api.ts';
import type { Layout } from '../src/lib/layout/model.ts';
import { gridPage, importText, openLayout, tile, waitEvent } from './layout-helpers.ts';

const folder = process.env.TD_SHOTS;
const mediaText = readFileSync(join(import.meta.dirname, '..', '..', '..', 'schema', 'examples', 'media.json'), 'utf8');

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

async function drawsLiveTiles(page: Page) {
  await page.evaluate((capabilities) => window.touchdeckDemo?.device('mock0001', { capabilities }), LIVE_CAPS);
}

async function stored(page: Page): Promise<Layout> {
  const record = (await page.evaluate(async () => (await fetch('/api/v1/test/devices/mock0001/record')).json())) as { layouts: { landscape: Layout } };
  return record.layouts.landscape;
}

interface LiveSet {
  tile: string;
  stream: string;
  type: string;
  value: number | string | null;
}

async function held(page: Page, tile: string): Promise<Record<string, LiveSet['value']>> {
  const sets = (await page.evaluate(async () => (await fetch('/api/v1/test/devices/mock0001/sets')).json())) as LiveSet[];
  return Object.fromEntries(sets.filter((set) => set.tile === tile).map((set) => [set.stream, set.value]));
}

async function volumeLevel(page: Page): Promise<{ level: number; muted: boolean }> {
  return page.evaluate(async () => (await fetch('/api/v1/test/integrations/volume')).json());
}

async function dragOnDevice(page: Page, pageId: string, tile: string, values: number[]): Promise<number[]> {
  return page.evaluate(
    async ({ pageId, tile, values }) => {
      const statuses: number[] = [];
      for (const [i, value] of values.entries()) {
        const phase = i === values.length - 1 ? 'end' : 'move';
        const sent = await fetch('/api/v1/test/devices/mock0001/input', { method: 'POST', body: JSON.stringify({ page: pageId, tile, stream: 'value', phase, value }) });
        statuses.push(sent.status);
      }
      return statuses;
    },
    { pageId, tile, values },
  );
}

const cell = (page: Page, row: number, col: number) => page.locator(`[data-testid="tile"][data-row="${row}"][data-col="${col}"]`);

async function addPreset(page: Page, integration: string, preset: string, row: number, col: number) {
  await page.locator(`[data-testid="empty-cell"][data-row="${row}"][data-col="${col}"]`).click();
  await page.getByTestId('add-presets').locator(`[data-id="${integration}"] [data-preset="${preset}"]`).click();
}

async function onShortcuts(page: Page) {
  await openLayout(page);
  await page.getByTestId('start-starter').click();
  await page.getByTestId('page-tab').filter({ hasText: 'Shortcuts' }).click();
}

test('a Volume bar from Add Tile reaches the Device as a bar a drag can set', async ({ page }) => {
  await onShortcuts(page);
  await drawsLiveTiles(page);
  await addPreset(page, 'volume', 'volume_bar', 2, 2);
  const placed = cell(page, 2, 2);
  await expect(placed).toHaveAttribute('aria-label', 'Volume bar');
  const level = placed.getByTestId('tile-level');
  await expect(level).toHaveAttribute('data-value', '64');
  await expect(level).toHaveAttribute('data-input', 'true');
  await expect(page.getByTestId('tile-shows-bar')).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByTestId('tile-icon')).toHaveCount(0);

  const drag = page.getByTestId('tile-input');
  await expect(drag).toBeChecked();
  await drag.click();
  await expect(level).toHaveAttribute('data-input', 'false');
  await drag.click();
  await expect(level).toHaveAttribute('data-input', 'true');

  await page.getByTestId('push').click();
  await waitEvent(page, 'pushed');
  const shortcuts = gridPage((await stored(page)).pages.find((p) => p.title === 'Shortcuts'));
  expect(shortcuts.tiles.find((t) => t.row === 2 && t.col === 2)).toMatchObject({
    label: 'Volume bar',
    icon: null,
    render: 'bar',
    streams: { value: 'number', muted: 'number' },
    input: true,
    action: { type: 'widget', source: 'volume', field: 'level', account: '' },
  });

  const bar = shortcuts.tiles.find((t) => t.row === 2 && t.col === 2)!;
  expect(await volumeLevel(page)).toEqual({ level: 0.64, muted: false });
  await expect.poll(() => held(page, bar.id)).toEqual({ value: 0.64, muted: 0 });
  expect(await dragOnDevice(page, shortcuts.id, bar.id, [0.5, 0.31, 0.3])).toEqual([204, 204, 204]);
  expect(await volumeLevel(page)).toEqual({ level: 0.3, muted: false });
  await expect.poll(() => held(page, bar.id)).toEqual({ value: 0.3, muted: 0 });
  expect(await dragOnDevice(page, shortcuts.id, bar.id, [1.7])).toEqual([204]);
  expect(await volumeLevel(page)).toEqual({ level: 1, muted: false });
});

test('a bar turned back into its value drops the drag', async ({ page }) => {
  await onShortcuts(page);
  await drawsLiveTiles(page);
  await addPreset(page, 'volume', 'volume_bar', 2, 2);
  await page.getByTestId('tile-shows-plain').click();
  await expect(cell(page, 2, 2).getByTestId('tile-level')).toHaveCount(0);
  await expect(page.getByTestId('tile-input')).toHaveCount(0);
  await page.getByTestId('push').click();
  await waitEvent(page, 'pushed');
  const shortcuts = gridPage((await stored(page)).pages.find((p) => p.title === 'Shortcuts'));
  const value = shortcuts.tiles.find((t) => t.row === 2 && t.col === 2);
  expect(value).toMatchObject({ action: { type: 'widget', source: 'volume', field: 'level' } });
  expect(value).not.toHaveProperty('render');
  expect(value).not.toHaveProperty('input');
  expect(await dragOnDevice(page, shortcuts.id, value!.id, [0.1])).toEqual([409]);
  expect(await volumeLevel(page)).toEqual({ level: 0.64, muted: false });
  expect(await held(page, value!.id)).toEqual({});
});

test('a Now playing Tile reaches the Device with the title, artist, cover and state streams', async ({ page }) => {
  await onShortcuts(page);
  await drawsLiveTiles(page);
  await addPreset(page, 'media', 'now_playing', 2, 3);
  const placed = cell(page, 2, 3);
  await expect(placed).toHaveAttribute('aria-label', 'Now playing');
  await expect(placed.getByTestId('tile-now-playing')).toHaveAttribute('data-idle', 'false');
  await expect(page.getByTestId('tile-shows-now-playing')).toHaveText('Now playing');
  await expect(page.getByTestId('tile-icon')).toHaveCount(0);
  await expect(page.getByTestId('tile-input')).toHaveCount(0);

  await page.getByTestId('push').click();
  await waitEvent(page, 'pushed');
  const shortcuts = gridPage((await stored(page)).pages.find((p) => p.title === 'Shortcuts'));
  const playing = shortcuts.tiles.find((t) => t.row === 2 && t.col === 3);
  expect(playing).toMatchObject({
    label: 'Now playing',
    icon: null,
    render: 'now-playing',
    streams: { title: 'text', artist: 'text', cover: 'image', state: 'number' },
    action: { type: 'widget', source: 'media', field: 'title', account: '' },
  });
  expect(playing).not.toHaveProperty('input');
  await expect.poll(() => held(page, playing!.id)).toEqual({ title: 'Night Drive', artist: 'Tidal Lines', cover: null, state: 2 });
});

test('without the Firmware, Volume and Now playing Presets are placed anyway and say Needs Firmware', async ({ page }) => {
  await onShortcuts(page);
  await page.locator('[data-testid="empty-cell"][data-row="2"][data-col="2"]').click();
  const presets = page.getByTestId('add-presets');
  const needs = /^Needs (Firmware \d+\.\d+|newer Firmware)$/;
  for (const [integration, preset, name] of [['volume', 'volume_bar', 'Volume bar'], ['media', 'now_playing', 'Now playing']]) {
    const button = presets.locator(`[data-id="${integration}"] [data-preset="${preset}"]`);
    await expect(button.locator('.preset-needs')).toHaveText(needs);
    await expect(button.locator('.preset-name')).toHaveText(name);
    await expect(button).toHaveAttribute('aria-label', new RegExp(`^Add ${name}\\. ${needs.source.slice(1)}`));
  }
  await presets.locator('[data-id="volume"] [data-preset="volume_bar"]').click();

  const placed = cell(page, 2, 2);
  await expect(placed).toHaveAttribute('data-needs', needs);
  await expect(placed.getByTestId('tile-level')).toHaveCount(0);
  await expect(placed.locator('.tile-needs-label')).toHaveText('Volume bar');
  await expect(page.getByTestId('tile-shows-bar')).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByTestId('tile-shows-hint')).toHaveText(/^Needs Firmware \d+\.\d+\.$|Firmware can't change this yet\.$/);

  await drawsLiveTiles(page);
  await expect(placed).not.toHaveAttribute('data-needs');
  await expect(placed.getByTestId('tile-level')).toHaveAttribute('data-value', '64');
});

test('the Integrations page lists Volume and Media, and Add Tile previews their Tiles as the Device draws them', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.getByRole('button', { name: 'Integrations', exact: true }).click();
  const volume = page.locator('[data-testid="integration"][data-id="volume"]');
  const media = page.locator('[data-testid="integration"][data-id="media"]');
  await expect(volume).toContainText("Shows and sets this PC's volume.");
  await expect(media).toContainText("Shows what's playing on this PC, with play, pause and skip.");
  await onShortcuts(page);
  await drawsLiveTiles(page);
  await page.locator('[data-testid="empty-cell"][data-row="2"][data-col="2"]').click();
  const presets = page.getByTestId('add-presets');
  await expect(presets.locator('[data-id="volume"] [data-preset="volume_bar"]')).toHaveAttribute('data-look', 'level');
  await expect(presets.locator('[data-id="volume"] [data-preset="volume_bar"]').getByTestId('tile-level')).toHaveAttribute('data-value', '64');
  await expect(presets.locator('[data-id="media"] [data-preset="now_playing"]')).toHaveAttribute('data-look', 'now-playing');
  await expect(presets.locator('[data-id="media"] [data-preset="now_playing"]').getByTestId('tile-now-playing')).toBeVisible();
});

for (const theme of ['light', 'dark'] as const) {
  test.describe(theme, () => {
    test.skip(!folder, 'set TD_SHOTS to a folder to take screenshots');
    test.use({ colorScheme: theme });

    test('Volume and Now playing Tiles in the editor and the deck preview', async ({ page }) => {
      await openLayout(page);
      await drawsLiveTiles(page);
      await importText(page, mediaText, 'media.json');
      await waitEvent(page, 'opened');
      await tile(page, 'volume').click();
      await expect(page.getByTestId('tile-input')).toBeChecked();
      await page.mouse.move(0, 0);
      await page.screenshot({ path: join(folder!, `media-volume-${theme}.png`) });
      await tile(page, 'playing').click();
      await expect(page.getByTestId('tile-shows-now-playing')).toBeVisible();
      await page.mouse.move(0, 0);
      await page.screenshot({ path: join(folder!, `media-now-playing-${theme}.png`) });
      await page.getByTestId('preview').click();
      await expect(page.getByTestId('deck-preview')).toHaveAttribute('data-mode', 'preview');
      await page.mouse.move(0, 0);
      await page.screenshot({ path: join(folder!, `media-preview-${theme}.png`) });
      await page.evaluate(() => window.touchdeckDemo?.offline('mock0001'));
      await expect(tile(page, 'playing').getByTestId('tile-now-playing')).toHaveAttribute('data-stale', 'true');
      await page.screenshot({ path: join(folder!, `media-preview-stale-${theme}.png`) });
    });

    test('Volume and Media rows on the Integrations page', async ({ page }) => {
      await page.goto('/');
      await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
      await page.getByRole('button', { name: 'Integrations', exact: true }).click();
      await expect(page.locator('[data-testid="integration"][data-id="media"]')).toBeVisible();
      await page.evaluate(() => document.fonts.ready);
      await page.mouse.move(0, 0);
      await page.screenshot({ path: join(folder!, `media-integrations-${theme}.png`) });
      await onShortcuts(page);
      await drawsLiveTiles(page);
      await page.locator('[data-testid="empty-cell"][data-row="2"][data-col="2"]').click();
      await expect(page.locator('[data-testid="add-presets"] [data-preset="volume_bar"]')).not.toHaveAttribute('data-needs');
      await page.mouse.move(0, 0);
      await page.screenshot({ path: join(folder!, `media-integrations-live-${theme}.png`) });
    });
  });
}
