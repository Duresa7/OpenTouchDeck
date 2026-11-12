import { readFileSync } from 'node:fs';
import { expect, type Locator, type Page } from '@playwright/test';
import { geometry, screenSize, type Orientation } from '../src/lib/deck/geometry.ts';
import { parseLayout, type GridPage, type Layout, type Page as LayoutPage } from '../src/lib/layout/model.ts';

export async function openLayout(page: Page, path = '/') {
  await page.addInitScript(() => {
    delete window.showSaveFilePicker;
  });
  await page.goto(path);
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.getByRole('button', { name: 'Layout', exact: true }).click();
  await expect(page.getByTestId('editor')).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}

export const status = (page: Page) => page.getByTestId('status');

/** A Page's entry in the Pages column. */
export const pageEntry = (page: Page, id: string) => page.locator(`[data-testid="page-entry"][data-page="${id}"]`);

/** The Page names in the Pages column, in order. */
export const pageTitles = (page: Page) => page.getByTestId('page-entry-title');

/** Brings the Pages column out if it's folded, or hidden by the Details panel in an editor too narrow for both. */
export async function showPages(page: Page) {
  const toggle = page.getByTestId('pages-toggle');
  if ((await toggle.getAttribute('aria-expanded')) === 'false') await toggle.click();
  await expect(page.getByTestId('page-strip')).toHaveAttribute('data-open', 'true');
  await expect(page.getByTestId('page-strip')).not.toHaveAttribute('inert');
}

export async function waitEvent(page: Page, event: string, timeout = 10_000): Promise<Locator> {
  await expect(status(page)).toHaveAttribute('data-event', event, { timeout });
  return status(page);
}

export async function importText(page: Page, text: string, name: string) {
  await page.getByTestId('import-file').setInputFiles({ name, mimeType: 'application/json', buffer: Buffer.from(text) });
}

export async function menu(page: Page, item: 'menu-push' | 'pull' | 'import' | 'export' | 'menu-starter' | 'menu-new' | 'prune') {
  await page.getByTestId('layout-menu').click();
  await page.getByTestId(item).click();
}

/** The Settings page's categories, as its sidebar's test ids name them. */
export type SettingsCategory = 'general' | 'browsers' | 'troubleshooting' | 'errors' | 'about';

export const settingsPage = (page: Page) => page.getByTestId('settings');

/** Goes to Settings with the nav's Settings button, which always opens General, then to a category. */
export async function openSettings(page: Page, category: SettingsCategory = 'general'): Promise<Locator> {
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(settingsPage(page)).toHaveAttribute('data-section', 'general');
  if (category !== 'general') await page.getByTestId(`settings-nav-${category}`).click();
  await expect(settingsPage(page)).toHaveAttribute('data-section', category);
  return settingsPage(page);
}

/**
 * Opens a Device's settings, the `device-panel`: Settings from the nav, then the Device in its sidebar. Without an id
 * it opens the Device the editor shows, or the first Device.
 */
export async function openDeviceSettings(page: Page, device?: string): Promise<Locator> {
  const editor = page.getByTestId('editor');
  const layoutButton = page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Layout', exact: true });
  if (!device && (await layoutButton.getAttribute('aria-current')) === 'page') await expect(editor).toBeVisible();
  const id = device ?? ((await editor.count()) > 0 ? await editor.getAttribute('data-device-id') : null);
  await openSettings(page);
  const items = page.getByTestId('settings-nav-device');
  await (id ? items.and(page.locator(`[data-device="${id}"]`)) : items.first()).click();
  const panel = page.getByTestId('device-panel');
  await expect(panel).toBeVisible();
  if (id) await expect(panel).toHaveAttribute('data-device', id);
  return panel;
}

/** The card an error pops up, found by its words. */
export const errorCard = (page: Page, text: string | RegExp) => page.getByTestId('error-card').filter({ hasText: text });

/**
 * Opens Settings → Errors from an error card's Details button, and returns the newest entry's detail.
 */
export async function errorDetail(page: Page, text: string | RegExp): Promise<Locator> {
  await errorCard(page, text).getByRole('button', { name: 'Details' }).click();
  await expect(settingsPage(page)).toHaveAttribute('data-section', 'errors');
  const entry = page.getByTestId('error-entry').first();
  await expect(entry).toContainText(text);
  return entry.getByTestId('error-detail');
}

export async function exportText(page: Page): Promise<string> {
  const download = page.waitForEvent('download');
  await menu(page, 'export');
  return readFileSync(await (await download).path(), 'utf8');
}

export const tile = (page: Page, id: string) => page.locator(`[data-testid="tile"][data-id="${id}"]`);

/** A square of the Details panel's grid picker: clicking it sets the grid to that many rows and columns. */
export const gridCell = (page: Page, rows: number, cols: number) => page.locator(`[data-testid="grid-cell"][data-rows="${rows}"][data-cols="${cols}"]`);

export async function placeOf(page: Page, id: string) {
  const el = tile(page, id);
  const read = async (name: string) => Number(await el.getAttribute(name));
  return { row: await read('data-row'), col: await read('data-col'), rowSpan: await read('data-row-span'), colSpan: await read('data-col-span') };
}

export async function cellPoint(page: Page, row: number, col: number) {
  const preview = page.getByTestId('deck-preview');
  const orientation: Orientation = (await preview.getAttribute('data-orientation')) === 'portrait' ? 'portrait' : 'landscape';
  const g = geometry(screenSize(orientation), (await preview.getAttribute('data-full-screen')) === 'true');
  const grid = page.getByTestId('grid');
  const rows = Number(await grid.getAttribute('data-rows'));
  const cols = Number(await grid.getAttribute('data-cols'));
  const box = (await page.getByTestId('deck-screen').boundingBox())!;
  const s = box.width / g.screen.w;
  const cw = (g.grid.w + g.gap) / cols;
  const ch = (g.grid.h + g.gap) / rows;
  return { x: box.x + (g.grid.x + col * cw + (cw - g.gap) / 2) * s, y: box.y + (g.grid.y + row * ch + (ch - g.gap) / 2) * s };
}

export async function drag(page: Page, id: string, row: number, col: number, how: 'move' | 'resize') {
  const at = await placeOf(page, id);
  let from: { x: number; y: number };
  if (how === 'resize') {
    await tile(page, id).click();
    const box = (await page.getByTestId('resize-handle').boundingBox())!;
    from = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
  } else {
    from = await cellPoint(page, at.row, at.col);
  }
  const to = await cellPoint(page, row, col);
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(from.x + (to.x - from.x) / 4, from.y + (to.y - from.y) / 4, { steps: 3 });
  await page.mouse.move(to.x, to.y, { steps: 6 });
  await page.mouse.up();
}

export async function swipe(page: Page, dx: number) {
  const box = (await page.getByTestId('deck-screen').boundingBox())!;
  const s = Number(await page.getByTestId('deck-screen').getAttribute('data-scale'));
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx * s, y, { steps: 8 });
  await page.mouse.up();
}

export function sorted(layout: Layout) {
  return layout.pages.map((page) => (page.kind === 'grid' ? { ...page, tiles: [...page.tiles].sort((a, b) => a.id.localeCompare(b.id)) } : page));
}

export function layoutOf(text: string): Layout {
  const parsed = parseLayout(text);
  if (!parsed.ok) throw new Error(`The Layout breaks ${parsed.errors.map((e) => e.key).join(', ')}`);
  return parsed.layout;
}

export function gridPage(page: LayoutPage | undefined): GridPage {
  if (page?.kind !== 'grid') throw new Error(`${page?.id} is not a grid page`);
  return page;
}
