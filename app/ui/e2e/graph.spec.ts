import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import type { Capabilities } from '../src/lib/companion/api.ts';
import { exportText, gridPage, layoutOf, openLayout, tile } from './layout-helpers.ts';

const folder = process.env.TD_SHOTS;

const GRAPH_CAPS: Capabilities = {
  screen: [1024, 600],
  grid: [8, 8],
  pages: 8,
  tiles: 64,
  layout_bytes: 32768,
  layout_schemas: [1, 2],
  storage_free: 1_000_000,
  tile_kinds: ['static', 'graph'],
  asset_bytes: 12300,
  formats: ['tdi1'],
  image: [64, 64],
  photo_bytes: 0,
  live_hz: 2,
  live_total_hz: 16,
  series: 32,
  text_bytes: 0,
  live_images: 0,
};

async function drawsGraphs(page: Page, caps: Capabilities = GRAPH_CAPS) {
  await page.evaluate((capabilities) => window.touchdeckDemo?.device('mock0001', { capabilities }), caps);
}

async function starterWithGraphs(page: Page) {
  await openLayout(page);
  await page.getByTestId('start-starter').click();
  await drawsGraphs(page);
}

test('Graph waits for Firmware that draws graphs, then binds a Tile to any numeric stream', async ({ page }) => {
  await openLayout(page);
  await page.getByTestId('start-starter').click();
  await tile(page, 'next').click();
  await expect(page.getByTestId('tile-shows-plain')).toHaveText('Label');
  await expect(page.getByTestId('tile-shows-graph')).toBeDisabled();
  await expect(page.getByTestId('tile-shows-hint')).toHaveText(/^Needs Firmware \d+\.\d+\.$|Firmware can't change this yet\.$/);

  await drawsGraphs(page);
  await expect(page.getByTestId('tile-shows-hint')).toHaveCount(0);
  await page.getByTestId('tile-shows-graph').click();
  const stream = page.getByTestId('graph-stream');
  const first = await stream.locator('optgroup').first().locator('option').first().getAttribute('value');
  await expect(stream).toHaveValue(first ?? 'a first option');
  expect(await stream.locator('optgroup').evaluateAll((groups) => groups.map((g) => g.getAttribute('label')))).toEqual(['CLIProxyAPI', 'This PC', 'Volume', 'Media']);
  await expect(stream.locator('optgroup[label="This PC"] option')).toHaveCount(10);
  await stream.selectOption('this_pc/cpu_pct');
  await expect(page.getByTestId('tile-icon')).toHaveCount(0);
  const graph = tile(page, 'next').getByTestId('tile-graph');
  await expect(graph).toHaveAttribute('data-points', '32');
  await expect(graph).toHaveAttribute('data-stale', 'false');

  await stream.selectOption({ label: 'GPU usage' });
  const exported = layoutOf(await exportText(page));
  expect(exported.schema).toBe(2);
  const next = gridPage(exported.pages[0]).tiles.find((t) => t.id === 'next');
  expect(next).toMatchObject({ render: 'graph', streams: { value: 'number' }, icon: null, action: { type: 'widget', source: 'this_pc', field: 'gpu_pct', account: '' } });

  await page.getByTestId('tile-shows-plain').click();
  await expect(page.getByTestId('tile-shows-plain')).toHaveText('Value');
  await expect(tile(page, 'next').getByTestId('tile-graph')).toHaveCount(0);
  await expect(page.getByTestId('tile-icon')).toBeVisible();
});

test('the preview keeps as many samples as the Device does', async ({ page }) => {
  await starterWithGraphs(page);
  await drawsGraphs(page, { ...GRAPH_CAPS, series: 12 });
  await tile(page, 'prev').click();
  await page.getByTestId('tile-shows-graph').click();
  await expect(tile(page, 'prev').getByTestId('tile-graph')).toHaveAttribute('data-points', '12');
});

test('a This PC Preset placed from Add Tile is a graph of its stream', async ({ page }) => {
  await starterWithGraphs(page);
  await page.getByTestId('page-entry').filter({ hasText: 'Shortcuts' }).click();
  await page.locator('[data-testid="empty-cell"][data-row="2"][data-col="2"]').click();
  await page.getByTestId('add-presets').locator('[data-id="this_pc"] [data-preset="ram_pct"]').click();
  const placed = page.locator('[data-testid="tile"][data-row="2"][data-col="2"]');
  await expect(placed).toHaveAttribute('aria-label', 'RAM usage');
  await expect(placed.getByTestId('tile-graph')).toHaveAttribute('data-points', '32');
  await expect(page.getByTestId('graph-stream')).toHaveValue('this_pc/ram_pct');
});

test('without graph Firmware a This PC Preset stays a value Tile', async ({ page }) => {
  await openLayout(page);
  await page.getByTestId('start-starter').click();
  await page.getByTestId('page-entry').filter({ hasText: 'Shortcuts' }).click();
  await page.locator('[data-testid="empty-cell"][data-row="2"][data-col="2"]').click();
  await page.getByTestId('add-presets').locator('[data-id="this_pc"] [data-preset="cpu_pct"]').click();
  const placed = page.locator('[data-testid="tile"][data-row="2"][data-col="2"]');
  await expect(placed).toHaveAttribute('data-action', 'widget');
  await expect(placed.getByTestId('tile-graph')).toHaveCount(0);
});

test('a Live tile offers its own Integration and CLIProxyAPI by name, with its readings by name', async ({ page }) => {
  await openLayout(page);
  await page.getByTestId('start-starter').click();
  await page.getByTestId('page-entry').filter({ hasText: 'Shortcuts' }).click();
  await page.locator('[data-testid="empty-cell"][data-row="2"][data-col="2"]').click();
  await page.getByTestId('add-presets').locator('[data-id="this_pc"] [data-preset="cpu_pct"]').click();
  const source = page.getByTestId('action-source');
  await expect(source).toHaveValue('this_pc');
  expect((await source.locator('option').allTextContents()).sort()).toEqual(['CLIProxyAPI', 'This PC']);
  const field = page.getByTestId('action-field');
  await expect(field).toHaveValue('cpu_pct');
  await expect(field.locator('option:checked')).toHaveText('CPU usage');
  await expect(field.locator('option')).toHaveCount(10);
  await expect(page.getByTestId('action-account')).toHaveCount(0);

  await source.selectOption({ label: 'CLIProxyAPI' });
  await expect(source).toHaveValue('cliproxyapi');
  await expect(page.getByTestId('action-account')).toBeVisible();
  const action = gridPage(layoutOf(await exportText(page)).pages[1]).tiles.find((t) => t.row === 2 && t.col === 2)?.action;
  expect(action).toMatchObject({ type: 'widget', source: 'cliproxyapi', account: '' });
  expect(action).not.toMatchObject({ field: 'cpu_pct' });
});

test('an offline Device shows its graphs stale, at 60% with the unavailable marker', async ({ page }) => {
  await starterWithGraphs(page);
  await tile(page, 'next').click();
  await page.getByTestId('tile-shows-graph').click();
  const graph = tile(page, 'next').getByTestId('tile-graph');
  await expect(graph).toHaveAttribute('data-stale', 'false');
  await expect(tile(page, 'next')).not.toContainText('--');
  await page.evaluate(() => window.touchdeckDemo?.offline('mock0001'));
  await expect(graph).toHaveAttribute('data-stale', 'true');
  await expect(graph).toHaveCSS('opacity', '0.6');
  await expect(tile(page, 'next')).toContainText('--');
});

for (const theme of ['light', 'dark'] as const) {
  test.describe(theme, () => {
    test.skip(!folder, 'set TD_SHOTS to a folder to take screenshots');
    test.use({ colorScheme: theme });

    test('graph Tiles, one picked', async ({ page }) => {
      await starterWithGraphs(page);
      for (const id of ['prev', 'playpause']) {
        await tile(page, id).click();
        await page.getByTestId('tile-shows-graph').click();
      }
      await page.getByTestId('graph-stream').selectOption({ label: 'RAM usage' });
      await tile(page, 'next').click();
      await page.getByTestId('tile-shows-graph').click();
      await page.getByTestId('graph-stream').selectOption({ label: 'Network receive' });
      await page.mouse.move(0, 0);
      await page.screenshot({ path: join(folder!, `graph-${theme}.png`) });
      await page.evaluate(() => window.touchdeckDemo?.offline('mock0001'));
      await expect(tile(page, 'next').getByTestId('tile-graph')).toHaveAttribute('data-stale', 'true');
      await page.screenshot({ path: join(folder!, `graph-stale-${theme}.png`) });
    });
  });
}
