import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { fakeGif } from '../src/lib/companion/fake.ts';
import { GIF_CAPS } from '../src/lib/companion/samples.ts';
import type { Layout, Tile } from '../src/lib/layout/model.ts';
import { DESK, openStarter } from './inspector-helpers.ts';
import { errorDetail, gridPage, importText, status, tile, waitEvent } from './layout-helpers.ts';

const folder = process.env.TD_SHOTS;

async function shot(page: Page, name: string) {
  if (folder) await page.screenshot({ path: join(folder, `${name}.png`) });
}

const gifFile = (name: string, seed: number) => ({ name, mimeType: 'image/gif', buffer: Buffer.from(fakeGif({ seed, width: 48, height: 32 })) });

async function playsGifs(page: Page) {
  await page.evaluate((capabilities) => window.touchdeckDemo?.device('mock0001', { capabilities }), GIF_CAPS);
}

async function record(page: Page): Promise<Layout | null> {
  const read: { layouts: { landscape: Layout | null } } = await page.evaluate(async (id) => (await fetch(`/api/v1/test/devices/${id}/record`)).json(), DESK);
  return read.layouts.landscape;
}

async function addGifTile(page: Page, name = 'cat.gif'): Promise<string> {
  await page.locator('[data-testid="empty-cell"][data-row="2"][data-col="2"]').click();
  await page.getByTestId('add-tab-actions').click();
  await page.getByTestId('add-gif-file').setInputFiles(gifFile(name, 1));
  const id = await page.getByTestId('tile-fields').getAttribute('data-id');
  if (id === null) throw new Error('Add Tile placed no GIF Tile');
  return id;
}

const gifStatus = (page: Page) => page.getByTestId('gif-status');

test('Add Tile offers GIF only to a Device that plays GIFs', async ({ page }) => {
  await openStarter(page);
  await page.locator('[data-testid="empty-cell"][data-row="2"][data-col="2"]').click();
  await page.getByTestId('add-tab-actions').click();
  await expect(page.getByTestId('add-kind').first()).toBeVisible();
  await expect(page.getByTestId('add-gif')).toHaveCount(0);
  await tile(page, 'copy').click();
  await expect(page.getByTestId('tile-shows-gif')).toHaveCount(0);

  await playsGifs(page);
  await expect(page.getByTestId('tile-shows-gif')).toHaveText('GIF');
  await page.locator('[data-testid="empty-cell"][data-row="2"][data-col="2"]').click();
  await expect(page.getByTestId('add-gif')).toContainText('Plays an animated GIF');
  await shot(page, 'gif-choice');
});

test('a GIF Tile prepares, plays, refits and is sent; a second GIF waits its turn and can be cancelled', async ({ page }) => {
  await openStarter(page, '/?render-ms=1500');
  await playsGifs(page);
  const id = await addGifTile(page);
  await expect(gifStatus(page)).toHaveAttribute('data-state', 'preparing');
  await expect(gifStatus(page)).toContainText('Preparing GIF…');
  await expect(page.getByTestId('gif-name')).toHaveText('cat.gif');
  await expect(page.getByTestId('tile-shows-hint')).toHaveCount(0);
  await shot(page, 'gif-preparing');

  await tile(page, 'copy').click();
  await page.getByTestId('tile-shows-gif-file').setInputFiles(gifFile('dog.gif', 2));
  await expect(gifStatus(page)).toHaveAttribute('data-state', 'queued');
  await expect(gifStatus(page)).toContainText('Waiting for another GIF…');
  await expect(page.getByTestId('tile-shows-gif')).toHaveAttribute('aria-pressed', 'true');
  await shot(page, 'gif-queued');
  await page.getByTestId('gif-cancel').click();
  await expect(page.getByTestId('gif-field')).toHaveCount(0);
  await expect(page.getByTestId('tile-icon')).toBeVisible();
  await expect(tile(page, 'copy').getByTestId('tile-gif')).toHaveCount(0);

  await tile(page, id).click();
  await expect(gifStatus(page)).toHaveAttribute('data-state', 'ready', { timeout: 15_000 });
  await expect(gifStatus(page)).toHaveText(/^Fitted to Desk: \d+ × \d+, 10 fps\.$/);
  await expect(page.getByTestId('gif-choose')).toHaveText('Change GIF');
  await expect(page.getByTestId('tile-head')).toContainText('GIF on the Shortcuts Page');
  await expect(tile(page, id).getByTestId('gif-image')).toHaveAttribute('data-motion', 'playing');
  await shot(page, 'gif-ready');

  await page.getByTestId('gif-fit-cover').click();
  await expect(gifStatus(page)).toHaveAttribute('data-state', 'preparing');
  await expect(gifStatus(page)).toHaveAttribute('data-state', 'ready', { timeout: 15_000 });
  await expect(page.getByTestId('gif-fit-cover')).toHaveAttribute('aria-pressed', 'true');

  await page.getByTestId('tile-size-2x1').click();
  await expect(gifStatus(page)).toHaveAttribute('data-state', 'preparing');
  await expect(gifStatus(page)).toHaveAttribute('data-state', 'ready', { timeout: 15_000 });
  const created = await page.evaluate(() => window.touchdeckDemo!.assets.created());
  const fits = await page.evaluate(() => window.touchdeckDemo!.assets.fits());
  expect(fits[0]).toBe('contain');
  expect(fits.slice(-2)).toEqual(['cover', 'cover']);
  const targets = created.flatMap((source) => (source.kind === 'gif' ? [source.target] : []));
  expect(targets.at(-1)?.width).toBeGreaterThan(targets[0].width);
  expect(targets.at(-1)?.height).toBe(targets[0].height);

  await page.getByTestId('push').click();
  await expect(await waitEvent(page, 'pushed')).toHaveText('Sent to Desk');
  const sent = gridPage((await record(page))!.pages[1]).tiles.find((t: Tile) => t.id === id);
  expect(sent).toMatchObject({ render: 'gif', icon: null, colSpan: 2, asset: expect.stringMatching(/^sha256:[0-9a-f]{64}$/) });
});

test('a GIF that fails says why, keeps the raw reason under Details, and retries', async ({ page }) => {
  await openStarter(page);
  await playsGifs(page);
  await page.evaluate(() =>
    window.touchdeckDemo!.assets.failGif({ code: 'asset.too_large', message: 'This GIF is too long. Choose a shorter GIF.', details: { field: 'gif_frames', limit: 60, actual: 240, unit: 'frames' } }),
  );
  const id = await addGifTile(page, 'long.gif');
  await expect(gifStatus(page)).toHaveAttribute('data-state', 'failed');
  await expect(page.getByTestId('gif-error').locator('.field-error')).toHaveText('This GIF is too long for Desk. Choose a shorter GIF.');
  await page.getByTestId('gif-error').locator('summary').click();
  await expect(page.getByTestId('gif-error').locator('details p')).toContainText('"field":"gif_frames"');
  await shot(page, 'gif-error');

  await page.getByTestId('gif-retry').click();
  await expect(gifStatus(page)).toHaveAttribute('data-state', 'ready', { timeout: 15_000 });
  await expect(tile(page, id).getByTestId('tile-gif')).toBeVisible();
});

test('a Page with more GIF Tiles than the Device plays is refused when sent, and the Device keeps its Layout', async ({ page }) => {
  await openStarter(page);
  await playsGifs(page);
  const before = await record(page);
  const gifs = Array.from({ length: 5 }, (_, i) => ({
    id: `gif${i + 1}`,
    label: 'GIF',
    row: Math.floor(i / 3),
    col: i % 3,
    rowSpan: 1,
    colSpan: 1,
    icon: null,
    render: 'gif',
    asset: `sha256:${String(i + 1).repeat(64)}`,
    action: { type: 'none' },
  }));
  const layout = { schema: 2, name: 'Too many GIFs', orientation: 'landscape', pages: [{ id: 'gifs', title: 'GIFs', kind: 'grid', rows: 2, cols: 3, tiles: gifs }] };
  await importText(page, JSON.stringify(layout), 'gifs.json');
  await waitEvent(page, 'opened');
  await expect(tile(page, 'gif1').getByTestId('gif-missing')).toBeVisible();
  await page.getByTestId('push').click();
  await waitEvent(page, 'push-failed');
  const refused = 'This Page has too many animations for Desk. Remove an animated Tile.';
  await expect(status(page).locator('.editor-status-text')).toHaveText(refused);
  await shot(page, 'gif-refused');
  expect(await record(page)).toEqual(before);
  await expect(await errorDetail(page, refused)).toContainText('layout.animation_limit');
});

test('with reduced motion the editor shows each GIF paused on its first frame', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await openStarter(page);
  await playsGifs(page);
  const id = await addGifTile(page);
  await expect(gifStatus(page)).toHaveAttribute('data-state', 'ready', { timeout: 15_000 });
  const canvas = tile(page, id).getByTestId('gif-image');
  await expect(canvas).toHaveAttribute('data-motion', 'poster');
  await expect(page.getByTestId('gif-field').getByTestId('gif-image')).toHaveAttribute('data-motion', 'poster');
  await page.getByTestId('preview').click();
  await expect(canvas).toHaveAttribute('data-motion', 'poster');
  await shot(page, 'gif-reduced-motion');

  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await expect(canvas).toHaveAttribute('data-motion', 'playing');
});
