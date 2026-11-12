import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { counts, validateSource, type Layout } from '../src/lib/layout/model.ts';
import { worstCase } from './fixtures/worst-case.ts';
import { drag, errorCard, errorDetail, exportText, gridCell, gridPage, importText, layoutOf, menu, openLayout, pageEntry, pageTitles, placeOf, showPages, sorted, status, swipe, tile, waitEvent } from './layout-helpers.ts';

const examples = join(import.meta.dirname, '..', '..', '..', 'schema', 'examples');
const devText = readFileSync(join(examples, 'dev.json'), 'utf8');
const dev = layoutOf(devText);
const first = gridPage(dev.pages[0]);
const folder = process.env.TD_SHOTS;

async function record(page: Page, device = 'mock0001') {
  return page.evaluate(async (id) => (await fetch(`/api/v1/test/devices/${id}/record`)).json(), device) as Promise<{ layouts: { landscape: Layout | null; portrait: Layout | null } }>;
}

async function storeOnDevice(page: Page, layout: Layout, device = 'mock0001') {
  await page.evaluate(async ([id, text]) => {
    await fetch(`/api/v1/test/devices/${id}/layouts/${JSON.parse(text).orientation}`, { method: 'PUT', body: text });
  }, [device, JSON.stringify(layout)] as const);
}

const drawn = (page: Page) => page.getByTestId('grid').getByTestId('tile').evaluateAll((els) => els.map((el) => el.getAttribute('data-id')));

async function importDev(page: Page) {
  await importText(page, devText, 'dev.json');
  await waitEvent(page, 'opened');
}

test.beforeEach(async ({ page }) => {
  await openLayout(page);
});

test('imports dev.json with an entry per Page in the Pages column and the first Page drawn', async ({ page }) => {
  await importDev(page);
  await expect(status(page)).toHaveText(`Imported “${dev.name}”: 4 Pages, 13 Tiles.`);
  const entries = await page.getByTestId('page-entry').evaluateAll((els) => els.map((el) => el.getAttribute('data-page')));
  expect(entries).toEqual(dev.pages.map((p) => p.id));
  await expect(page.getByTestId('page-entry').first()).toHaveAttribute('aria-current', 'true');
  expect((await drawn(page)).sort()).toEqual(first.tiles.map((t) => t.id).sort());
});

test('moves F5 onto a free cell, then refuses a move onto Mute and keeps F5 in place', async ({ page }) => {
  await importDev(page);
  const free = { row: first.rows - 1, col: first.cols - 1 };
  await drag(page, 'f5', free.row, free.col, 'move');
  await expect(tile(page, 'f5')).toHaveAttribute('data-row', String(free.row));
  await expect(tile(page, 'f5')).toHaveAttribute('data-col', String(free.col));
  const quiet = first.tiles.find((t) => t.id === 'quiet')!;
  await drag(page, 'f5', quiet.row, quiet.col, 'move');
  await expect(status(page)).toHaveAttribute('data-kind', 'error');
  await expect(status(page).locator('.editor-status-text')).toHaveText('That spot is taken by “Mute”. Pick an empty one.');
  expect(await placeOf(page, 'f5')).toMatchObject(free);
  await expect(await errorDetail(page, 'That spot is taken by “Mute”. Pick an empty one.')).toHaveText('overlaps quiet');
});

test("sends and shows the Device's Page and Tile counts", async ({ page }) => {
  await importDev(page);
  await expect(page.getByTestId('editor-head')).toHaveAttribute('data-save-state', 'unsent');
  await page.getByTestId('push').click();
  const { pages, tiles } = counts(dev);
  const sent = await waitEvent(page, 'pushed');
  await expect(sent).toHaveAttribute('data-pages', String(pages));
  await expect(sent).toHaveAttribute('data-tiles', String(tiles));
  await expect(sent).toHaveText('Sent to Desk');
  await expect(page.getByTestId('editor-head')).toHaveAttribute('data-save-state', 'saved');
  expect((await record(page)).layouts.landscape?.name).toBe(dev.name);
  await menu(page, 'prune');
  await expect(await waitEvent(page, 'assets-removed')).toHaveText('Removed 0 unused Assets from Desk');
});

test('grows Unused to 2 by 2, then refuses a size past the grid edge', async ({ page }) => {
  await importDev(page);
  await drag(page, 'f5', first.rows - 1, first.cols - 1, 'move');
  const spacer = first.tiles.find((t) => t.id === 'spacer')!;
  await drag(page, 'spacer', spacer.row + 1, spacer.col + 1, 'resize');
  await expect(tile(page, 'spacer')).toHaveAttribute('data-row-span', '2');
  await expect(tile(page, 'spacer')).toHaveAttribute('data-col-span', '2');
  await drag(page, 'spacer', spacer.row + 1, first.cols, 'resize');
  await expect(status(page).locator('.editor-status-text')).toHaveText(`That doesn't fit. This Page has ${first.rows} rows and ${first.cols} columns.`);
  expect(await placeOf(page, 'spacer')).toMatchObject({ rowSpan: 2, colSpan: 2 });
});

test("loads the Device's Layout after a confirm, and every Tile matches", async ({ page }) => {
  await importDev(page);
  await menu(page, 'pull');
  const dialog = page.getByRole('alertdialog');
  await expect(dialog).toContainText('Desk has “Demo Layout”');
  await expect(dialog).toContainText('(2 Pages, 16 Tiles). Replace the landscape Layout here with it?');
  await dialog.getByRole('button', { name: 'Replace' }).click();
  await expect(await waitEvent(page, 'pulled')).toHaveText('Loaded “Demo Layout” from Desk');
  const loaded = layoutOf(await exportText(page));
  const stored = (await record(page)).layouts.landscape!;
  expect(sorted(loaded)).toEqual(sorted(stored));
  await expect(page.getByTestId('editor-head')).toHaveAttribute('data-save-state', 'saved');
  expect((await drawn(page)).sort()).toEqual(gridPage(stored.pages[0]).tiles.map((t) => t.id).sort());
});

test('exports a file that passes the Layout rules', async ({ page }) => {
  await importDev(page);
  const text = await exportText(page);
  expect(validateSource(new TextEncoder().encode(text))).toEqual([]);
  expect(sorted(layoutOf(text))).toEqual(sorted(dev));
  await expect(await waitEvent(page, 'exported')).toHaveText('Exported “Development.json”');
});

test('renders a 64-Tile Page and times five sends', async ({ page }) => {
  const tiles = Array.from({ length: 64 }, (_, i) => ({
    id: `t${i}`,
    label: `T${i}`,
    row: Math.floor(i / 8),
    col: i % 8,
    rowSpan: 1,
    colSpan: 1,
    icon: null,
    action: { type: 'key', usage: 58 + (i % 12), mods: [] },
  }));
  const big = { schema: 1, name: 'Sixty-four', orientation: 'landscape', pages: [{ id: 'all', title: 'All', kind: 'grid', rows: 8, cols: 8, tiles }] };
  await page.evaluate(() => performance.clearMeasures('page-render'));
  await importText(page, JSON.stringify(big), 'sixty-four.json');
  await expect(page.getByTestId('grid').getByTestId('tile')).toHaveCount(64);
  await expect.poll(() => page.evaluate(() => performance.getEntriesByName('page-render').length)).toBeGreaterThan(0);
  const render = await page.evaluate(() => performance.getEntriesByName('page-render').map((entry) => Math.round(entry.duration * 10) / 10));
  await importDev(page);
  await page.evaluate(() => performance.clearMeasures('push'));
  const pushes = () => page.evaluate(() => performance.getEntriesByName('push').map((entry) => Math.round(entry.duration)));
  for (let n = 1; n <= 5; n++) {
    // Send to Device folds away once the Device has the Layout, so the sends after the first come from the menu.
    if (n === 1) await page.getByTestId('push').click();
    else await menu(page, 'menu-push');
    await expect.poll(async () => (await pushes()).length).toBe(n);
    await waitEvent(page, 'pushed');
  }
  const times = { page_render_64_ms: render, push_dev_ms: await pushes() };
  console.log(`layout timings ${JSON.stringify(times)}`);
  test.info().annotations.push({ type: 'timings', description: JSON.stringify(times) });
  expect(Math.max(...render)).toBeLessThan(200);
  expect(times.push_dev_ms).toHaveLength(5);
  expect(Math.max(...times.push_dev_ms)).toBeLessThan(500);
});

test('keeps a separate Layout for portrait, and both outlive a reload', async ({ page }) => {
  await page.getByTestId('start-starter').click();
  await expect(page.getByTestId('deck-preview')).toHaveAttribute('data-orientation', 'landscape');
  await page.getByTestId('orientation-portrait').click();
  await expect(page.getByTestId('start-layout')).toBeVisible();
  await page.getByTestId('start-new').click();
  await expect(page.getByTestId('deck-preview')).toHaveAttribute('data-orientation', 'portrait');
  await expect(pageTitles(page)).toHaveText(['Page 1']);
  await page.getByTestId('orientation-landscape').click();
  await expect(pageTitles(page)).toHaveText(['Media', 'Shortcuts']);
  await page.reload();
  await page.getByRole('button', { name: 'Layout', exact: true }).click();
  await expect(pageTitles(page)).toHaveText(['Media', 'Shortcuts']);
  await page.getByTestId('orientation-portrait').click();
  await expect(pageTitles(page)).toHaveText(['Page 1']);
});

test('tries Pages out like the Device: swipe, tap, a page jump and the arrow keys', async ({ page }) => {
  await importDev(page);
  await page.keyboard.press('p');
  const preview = page.getByTestId('deck-preview');
  await expect(preview).toHaveAttribute('data-mode', 'preview');
  await expect(page.getByTestId('tile-head')).toHaveCount(0);
  await swipe(page, -200);
  await expect(preview).toHaveAttribute('data-page', 'desktop');
  await swipe(page, 300);
  await expect(preview).toHaveAttribute('data-page', 'shortcuts');
  await expect(preview).not.toHaveAttribute('data-sliding');

  const shift = await tile(page, 'shift').boundingBox();
  await page.mouse.move(shift!.x + shift!.width / 2, shift!.y + shift!.height / 2);
  await page.mouse.down();
  await expect(tile(page, 'shift')).toHaveClass(/is-pressed/);
  await page.mouse.up();
  await expect(tile(page, 'shift')).toHaveClass(/is-one-shot/);

  await tile(page, 'accounts_jump').click();
  await expect(preview).toHaveAttribute('data-page', 'accounts');
  await expect(page.getByTestId('dashboard-page')).toBeVisible();
  await page.keyboard.press('ArrowLeft');
  await expect(preview).toHaveAttribute('data-page', 'usage');
  await page.keyboard.press('ArrowRight');
  await expect(preview).toHaveAttribute('data-page', 'accounts');

  await page.getByTestId('actual-size').click();
  const dpr = await page.evaluate(() => devicePixelRatio);
  await expect(page.getByTestId('deck-screen')).toHaveAttribute('data-scale', String(1 / dpr));
  await page.keyboard.press('p');
  await expect(preview).toHaveAttribute('data-mode', 'edit');
});

test('explains a refused change and a file it cannot open', async ({ page }) => {
  await importText(page, '{"schema": 1, "name": "Broken"', 'broken.json');
  await expect(status(page)).toHaveAttribute('data-kind', 'error');
  await expect(status(page).locator('.editor-status-text')).toHaveText('“broken.json” isn\'t a Layout file. Nothing changed.');
  await expect(errorCard(page, '“broken.json” isn\'t a Layout file. Nothing changed.')).toBeVisible();
  if (folder) await page.screenshot({ path: join(folder, 'layout-file-refused-light.png') });

  const tooMany = { ...dev, pages: Array.from({ length: 9 }, (_, i) => ({ id: `p${i}`, title: `P${i}`, kind: 'grid', rows: 1, cols: 1, tiles: [] })) };
  await importText(page, JSON.stringify(tooMany), 'nine.json');
  await expect(status(page).locator('.editor-status-text')).toHaveText('“nine.json” breaks a Layout rule. Nothing changed. See Details.');
  await expect(page.getByTestId('start-layout')).toBeVisible();

  await importDev(page);
  await gridCell(page, 1, first.cols).click();
  await expect(status(page)).toHaveAttribute('data-event', 'refused');
  await expect(status(page).locator('.editor-status-text')).toHaveText(/^The grid can't shrink past “.+”\. Move or remove it first\.$/);
  await expect(gridCell(page, first.rows, first.cols)).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByTestId('grid')).toHaveAttribute('data-rows', String(first.rows));
});

test('holds up to the worst realistic Layout', async ({ page }) => {
  const noSpill = async () => {
    const sizes = await page.evaluate(() => ({ page: document.documentElement.scrollWidth - document.documentElement.clientWidth, head: (() => {
      const head = document.querySelector('.editor-head')!;
      return head.scrollWidth - head.clientWidth;
    })() }));
    expect(sizes).toEqual({ page: 0, head: 0 });
    const device = (await page.locator('.device').boundingBox())!;
    const canvas = (await page.locator('.canvas').boundingBox())!;
    expect(device.x).toBeGreaterThanOrEqual(canvas.x);
    expect(device.x + device.width).toBeLessThanOrEqual(canvas.x + canvas.width + 0.5);
    expect(device.y + device.height).toBeLessThanOrEqual(canvas.y + canvas.height + 0.5);
    const foot = (await page.locator('.canvas-foot').boundingBox())!;
    const rail = (await page.locator('.nav-rail').boundingBox())!;
    expect(foot.y + foot.height).toBeLessThanOrEqual(canvas.y + canvas.height + 0.5);
    expect(canvas.x).toBeGreaterThanOrEqual(rail.x + rail.width);
    expect(await page.getByTestId('device-name').evaluate((el) => el.getBoundingClientRect().width)).toBeGreaterThan(30);
  };
  await importText(page, JSON.stringify(worstCase), 'worst-case.json');
  await waitEvent(page, 'opened');
  await expect(page.getByTestId('grid').getByTestId('tile')).toHaveCount(64);
  await noSpill();
  await tile(page, 't4').click();
  await expect(page.getByTestId('tile-title')).toHaveText('☕ Coffee break');
  if (folder) await page.screenshot({ path: join(folder, 'layout-worst-selected-light.png') });
  await page.getByTestId('preview').click();
  for (const id of ['column', 'accounts', 'meters', 'media', 'empty', 'jump']) {
    await page.locator(`[data-testid="page-entry"][data-page="${id}"]`).click();
    await expect(page.getByTestId('deck-preview')).toHaveAttribute('data-page', id);
    await noSpill();
  }
  await page.getByTestId('preview').click();
  await page.setViewportSize({ width: 960, height: 600 });
  // Too narrow for both panels: the column folds while Details is out, once the editor has measured its new width.
  await expect(page.getByTestId('page-strip')).toHaveAttribute('data-open', 'false');
  await showPages(page);
  await pageEntry(page, 'full').click();
  await noSpill();
  if (folder) await page.screenshot({ path: join(folder, 'layout-worst-small-light.png') });
  await page.setViewportSize({ width: 1180, height: 760 });
  await importText(page, JSON.stringify({ ...worstCase, orientation: 'portrait' }), 'worst-case-portrait.json');
  await expect(page.getByTestId('deck-preview')).toHaveAttribute('data-orientation', 'portrait');
  await noSpill();
});

test('a send that finds the Layout changed on the Device offers Replace and Keep mine', async ({ page }) => {
  await page.getByTestId('start-pull').click();
  await waitEvent(page, 'pulled');
  await tile(page, 'media0').click();
  await page.getByTestId('tile-clear').click();
  const elsewhere: Layout = { schema: 1, name: 'Changed elsewhere', orientation: 'landscape', pages: [{ id: 'x', title: 'X', kind: 'grid', rows: 1, cols: 2, tiles: [] }] };
  await storeOnDevice(page, elsewhere);

  await page.getByTestId('push').click();
  const dialog = page.getByRole('alertdialog');
  await expect(dialog).toContainText('Desk has “Changed elsewhere”');
  await expect(dialog.getByRole('button', { name: 'Replace' })).toBeVisible();
  if (folder) await page.screenshot({ path: join(folder, 'layout-412-light.png') });
  await dialog.getByRole('button', { name: 'Keep mine' }).click();
  await expect(await waitEvent(page, 'pushed')).toHaveAttribute('data-tiles', '15');
  expect((await record(page)).layouts.landscape?.name).toBe('Demo Layout');

  await storeOnDevice(page, elsewhere);
  await expect(page.getByTestId('push'), 'nothing changed since the send').toHaveAttribute('data-state', 'hidden');
  await menu(page, 'menu-push');
  await dialog.getByRole('button', { name: 'Replace' }).click();
  await expect(await waitEvent(page, 'pulled')).toHaveText('Loaded “Changed elsewhere” from Desk');
  await expect(pageTitles(page)).toHaveText(['X']);
});

test('Add Tile places a Tile with its kind and selects it; Cancel and Escape close it', async ({ page }) => {
  await page.getByTestId('start-starter').click();
  await page.locator('[data-testid="page-entry"][data-page="shortcuts"]').click();
  await page.locator('[data-testid="empty-cell"][data-row="2"][data-col="2"]').click();
  await expect(page.getByTestId('add-where')).toHaveText('Row 3, column 3 on the Shortcuts Page');
  await expect(page.locator('.cell-empty.is-target')).toHaveAttribute('data-col', '2');
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('add-where')).toHaveCount(0);
  await page.getByTestId('add-tile').click();
  await expect(page.getByTestId('add-where')).toHaveText('Row 3, column 3 on the Shortcuts Page');
  await page.getByTestId('add-cancel').click();
  await expect(page.getByTestId('add-where')).toHaveCount(0);

  await page.getByTestId('add-tile').click();
  await page.getByTestId('add-tab-actions').click();
  await page.locator('[data-testid="add-kind"][data-kind="consumer"]').click();
  await expect(page.locator('[data-testid="tile"][data-row="2"][data-col="2"]')).toHaveAttribute('data-action', 'consumer');
  await expect(page.getByTestId('tile-title')).toHaveText('Tile');
  await expect(page.getByTestId('tile-head')).toContainText('Media control on the Shortcuts Page');

  await page.getByTestId('add-tile').click();
  await page.locator('[data-testid="add-kind"][data-kind="launch"]').click();
  await expect(page.getByTestId('tile-head')).toContainText('App on the Shortcuts Page');
  await page.getByTestId('add-tile').click();
  await expect(page.getByTestId('add-where')).toHaveText('The Shortcuts Page is full, so it starts a new Page');
  await page.locator('[data-testid="add-kind"][data-kind="key"]').click();
  await expect(pageTitles(page)).toHaveText(['Media', 'Shortcuts', 'Page 3']);
  await expect(page.getByTestId('tile-head')).toContainText('Keyboard shortcut on the Page 3 Page');
});

test('a narrow header keeps only the Add Tile icon, centred in its button', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 600 });
  await page.getByTestId('start-starter').click();
  const button = page.getByTestId('add-tile');
  await expect(button.locator('.add-tile-words')).toHaveCSS('position', 'absolute');
  const box = (await button.boundingBox())!;
  const icon = (await button.locator('svg').boundingBox())!;
  expect(Math.abs(icon.x + icon.width / 2 - (box.x + box.width / 2))).toBeLessThanOrEqual(1);
});

test('the keyboard moves, resizes, clears and deselects a Tile', async ({ page }) => {
  await page.getByTestId('start-starter').click();
  await page.locator('[data-testid="page-entry"][data-page="shortcuts"]').click();
  await tile(page, 'desktop').focus();
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('tile-title')).toHaveText('Show desktop');
  await page.keyboard.press('ArrowRight');
  await expect(tile(page, 'desktop')).toHaveAttribute('data-col', '2');
  await page.keyboard.press('Shift+ArrowRight');
  await expect(tile(page, 'desktop')).toHaveAttribute('data-col-span', '2');
  await page.keyboard.press('ArrowUp');
  await expect(status(page).locator('.editor-status-text')).toHaveText('That spot is taken by “Save”. Pick an empty one.');
  await expect(tile(page, 'desktop')).toHaveAttribute('data-row', '2');
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('tile-head')).toHaveCount(0);
  await expect(page.getByTestId('page-details')).toBeVisible();
  await tile(page, 'desktop').click();
  await page.keyboard.press('Delete');
  await expect(tile(page, 'desktop')).toHaveCount(0);
});

test('a Device that is not connected can still be edited, but not sent to', async ({ page }) => {
  await page.evaluate(() => {
    if (!window.touchdeckDemo) throw new Error('The demo controls are missing.');
    window.touchdeckDemo.offline('mock0001');
  });
  await expect(page.getByTestId('disconnected')).toHaveText("Desk isn't connected. Plug it in with a USB cable. You can still edit its Layout.");
  await expect(page.getByTestId('start-pull')).toHaveAttribute('aria-disabled', 'true');
  await expect(page.getByTestId('start-pull')).toContainText('Plug in Desk to load its Layout.');
  await page.getByTestId('start-starter').click();
  await drag(page, 'mute', 1, 1, 'move');
  await expect(status(page)).toHaveAttribute('data-event', 'refused');
  await tile(page, 'mute').click();
  await page.getByTestId('tile-clear').click();
  await expect(tile(page, 'mute')).toHaveCount(0);
  await expect(page.getByTestId('push')).toHaveAttribute('aria-disabled', 'true');
  await page.getByTestId('push').hover();
  await expect(page.locator('.tooltip')).toHaveText("Desk isn't connected");
  await page.getByTestId('push').click({ force: true });
  await expect(status(page)).not.toHaveAttribute('data-event');
  expect((await record(page)).layouts.landscape?.name).toBe('Demo Layout');
});

test('focus is visible on Tiles, the Pages column and header controls', async ({ page }) => {
  await page.getByTestId('start-starter').click();
  const reach = async (testId: string, nth = 0) => {
    const el = page.getByTestId(testId).nth(nth);
    await el.focus();
    await page.keyboard.press('Tab');
    await page.keyboard.press('Shift+Tab');
    await expect(el).toBeFocused();
    return el.evaluate((node) => {
      const style = getComputedStyle(node);
      return { visible: node.matches(':focus-visible'), ring: style.outlineStyle !== 'none' || style.boxShadow.includes('inset') };
    });
  };
  await expect(page.getByTestId('push')).toHaveAttribute('data-state', 'send');
  for (const id of ['device-rename', 'orientation-landscape', 'preview', 'layout-menu', 'add-tile', 'push', 'details-toggle', 'pages-toggle', 'page-entry', 'page-menu', 'page-add']) {
    expect({ id, ...(await reach(id)) }).toEqual({ id, visible: true, ring: true });
  }
  expect(await reach('tile', 1)).toEqual({ visible: true, ring: true });
  if (folder) await page.screenshot({ path: join(folder, 'layout-focus-tile-light.png') });
  const ring = await page.getByTestId('tile').nth(1).evaluate((node) => getComputedStyle(node).boxShadow);
  const scale = Number(await page.getByTestId('deck-screen').getAttribute('data-scale'));
  expect(Number(/([\d.]+)px inset/.exec(ring)?.[1])).toBeCloseTo(2 / scale, 1);
});

test('hiding Details grows the Device, and picking a Tile, even the selected one, or Add Tile brings it back', async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 1000 });
  await importDev(page);
  const toggle = page.getByTestId('details-toggle');
  const inspector = page.getByTestId('inspector');
  const width = async () => (await page.locator('.device').boundingBox())!.width;
  await tile(page, 'f5').click();
  await expect(page.getByTestId('tile-title')).toBeVisible();
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  await expect(toggle).toHaveAttribute('aria-controls', 'editor-details');
  const rail = await page.locator('.nav-rail').boundingBox();
  const open = await width();

  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await expect(toggle).toHaveAccessibleName('Show details');
  await expect(page.getByTestId('editor')).toHaveClass(/\bis-details-hidden\b/);
  await expect(inspector).toBeHidden();
  await expect(page.locator('.inspector-resizer')).toHaveCount(0);
  await expect.poll(width, { message: 'the Device grows into the room' }).toBeGreaterThan(open + 100);
  expect(await page.locator('.nav-rail').boundingBox(), 'the rail stays put').toEqual(rail);

  const free = { row: first.rows - 1, col: first.cols - 1 };
  await drag(page, 'f5', free.row, free.col, 'move');
  await expect(tile(page, 'f5')).toHaveAttribute('data-row', String(free.row));
  await expect(inspector, 'moving a Tile leaves the panel hidden').toBeHidden();

  await tile(page, 'f5').click();
  await expect(inspector, 'clicking the selected Tile brings the panel back').toBeVisible();
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');

  await toggle.click();
  await expect(inspector).toBeHidden();
  await tile(page, 'f5').focus();
  await page.keyboard.press('Enter');
  await expect(inspector, 'Enter on a Tile brings the panel back').toBeVisible();

  await toggle.click();
  await expect(inspector).toBeHidden();
  await page.getByTestId('add-tile').click();
  await expect(inspector, 'Add Tile brings the panel back').toBeVisible();
  await expect(page.getByTestId('add-tile')).toHaveAttribute('aria-expanded', 'true');
  await toggle.click();
  await expect(page.getByTestId('add-tile'), 'hiding the panel ends Add Tile').toHaveAttribute('aria-expanded', 'false');
  await toggle.click();
  await expect(inspector).toBeVisible();
  await expect.poll(width, { message: 'the Device gives the room back' }).toBe(open);
});

test('the grid picker previews a size under the pointer and sets it with a click or the keyboard', async ({ page }) => {
  await importDev(page);
  const size = page.getByTestId('grid-size');
  const grid = page.getByTestId('grid');
  await expect(size).toHaveText(`${first.rows} rows × ${first.cols} columns`);
  await expect(gridCell(page, first.rows, first.cols)).toHaveAttribute('aria-pressed', 'true');
  await expect(gridCell(page, first.rows, first.cols)).toHaveAttribute('tabindex', '0');
  await expect(page.getByTestId('grid-picker').locator('[tabindex="0"]')).toHaveCount(1);

  await gridCell(page, 8, 8).hover();
  await expect(size).toHaveText('8 rows × 8 columns');
  await expect(gridCell(page, 8, 8)).toHaveAttribute('data-lit', 'preview');
  await page.getByTestId('page-name').hover();
  await expect(size, 'leaving the board drops the preview').toHaveText(`${first.rows} rows × ${first.cols} columns`);
  await expect(gridCell(page, 8, 8)).not.toHaveAttribute('data-lit');

  await gridCell(page, 5, 6).click();
  await expect(grid).toHaveAttribute('data-rows', '5');
  await expect(grid).toHaveAttribute('data-cols', '6');
  await expect(gridCell(page, 5, 6)).toHaveAttribute('aria-pressed', 'true');

  await gridCell(page, 5, 6).focus();
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('ArrowRight');
  await expect(gridCell(page, 6, 7)).toBeFocused();
  await expect(size).toHaveText('6 rows × 7 columns');
  await expect(grid, 'moving focus only previews').toHaveAttribute('data-rows', '5');
  await page.keyboard.press('Enter');
  await expect(grid).toHaveAttribute('data-rows', '6');
  await expect(grid).toHaveAttribute('data-cols', '7');
  await page.keyboard.press('End');
  await expect(gridCell(page, 8, 8)).toBeFocused();
  await page.keyboard.press('Home');
  await expect(gridCell(page, 1, 1)).toBeFocused();
  await expect(gridCell(page, 1, 1)).toHaveAttribute('tabindex', '0');
});

test('a Page’s menu, from its button or a right-click, removes the Page after asking once more', async ({ page }) => {
  await importDev(page);
  const remove = page.getByTestId('page-remove');
  const menuButton = (id: string) => page.locator(`.page-entry[data-page="${id}"]`).getByTestId('page-menu');
  await expect(menuButton('desktop')).toHaveCSS('opacity', '0');
  await pageEntry(page, 'desktop').hover();
  await expect(menuButton('desktop'), 'the menu button shows on hover').toHaveCSS('opacity', '1');
  await expect(menuButton('desktop')).toHaveAccessibleName('Options for “Desktop”');
  await menuButton('desktop').click();
  await expect(page.getByRole('menu', { name: 'Options for “Desktop”' })).toBeVisible();
  await expect(remove).toHaveText('Remove Page');
  await remove.click();
  await expect(remove).toContainText('Remove “Desktop”?');
  await expect(remove).toContainText('Click again to remove');
  await expect(page.getByTestId('page-entry'), 'the first click only asks').toHaveCount(dev.pages.length);
  await remove.click();
  await expect(page.getByRole('menu')).toHaveCount(0);
  await expect(page.getByTestId('page-entry')).toHaveCount(dev.pages.length - 1);
  await expect(pageEntry(page, 'desktop')).toHaveCount(0);

  await pageEntry(page, 'usage').click({ button: 'right' });
  await expect(page.getByRole('menu', { name: 'Options for “Usage”' })).toBeVisible();
  await remove.click();
  await expect(remove).toContainText('Remove “Usage”?');
  await page.keyboard.press('Escape');
  await expect(page.getByRole('menu')).toHaveCount(0);
  await pageEntry(page, 'usage').click({ button: 'right' });
  await expect(remove, 'closing the menu disarms it').toHaveText('Remove Page');
  await page.keyboard.press('Escape');
  await expect(page.getByRole('menu')).toHaveCount(0);

  await pageEntry(page, 'accounts').focus();
  await page.keyboard.press('Tab');
  await expect(menuButton('accounts')).toBeFocused();
  await expect(menuButton('accounts'), 'the menu button shows on keyboard focus').toHaveCSS('opacity', '1');
  await page.keyboard.press('Enter');
  await expect(remove).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(remove).toContainText('Remove “Accounts”?');
  await page.keyboard.press('Enter');
  await expect(pageTitles(page)).toHaveText(['Shortcuts', 'Usage']);
  await expect(pageEntry(page, 'shortcuts'), 'focus goes to the Page shown, not the window').toBeFocused();

  await menu(page, 'menu-new');
  await expect(page.getByTestId('page-entry')).toHaveCount(1);
  await pageEntry(page, 'page1').click({ button: 'right' });
  await expect(remove).toHaveAttribute('data-disabled');
  await expect(remove).toContainText('A Layout needs at least one Page');
  await remove.click({ force: true });
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('page-entry')).toHaveCount(1);
  await expect(page.getByTestId('page-details'), 'the Details panel no longer removes Pages').not.toContainText('Remove');
});

test('each Page’s thumbnail draws its Tiles at the Device’s shape and follows edits', async ({ page }) => {
  await importDev(page);
  const thumb = (id: string) => pageEntry(page, id).getByTestId('page-thumb');
  const ratio = (id: string) => thumb(id).evaluate((el) => el.getBoundingClientRect().width / el.getBoundingClientRect().height);
  expect(await ratio('shortcuts')).toBeCloseTo(1024 / 600, 1);
  await expect(thumb('shortcuts').locator('.thumb-tile')).toHaveCount(first.tiles.length);
  await expect(pageEntry(page, 'shortcuts')).toHaveAttribute('aria-current', 'true');
  const ring = await thumb('shortcuts').evaluate((el) => getComputedStyle(el).boxShadow);
  expect(ring, 'a 2px accent ring with a 2px gap').toMatch(/0px 0px 0px 2px, rgb\([^)]+\) 0px 0px 0px 4px/);
  await expect(thumb('desktop')).toHaveCSS('box-shadow', 'none');

  await tile(page, 'f5').click();
  await page.getByTestId('tile-clear').click();
  await expect(thumb('shortcuts').locator('.thumb-tile'), 'the thumbnail follows a cleared Tile').toHaveCount(first.tiles.length - 1);
  await page.getByTestId('page-name').fill('Keys');
  await page.getByTestId('page-name').press('Enter');
  await expect(pageEntry(page, 'shortcuts').getByTestId('page-entry-title')).toHaveText('Keys');
  await expect(pageEntry(page, 'shortcuts')).toHaveAttribute('title', 'Keys');

  const usage = pageEntry(page, 'accounts');
  await expect(usage.locator('.page-tag')).toHaveText('Usage');
  await expect(usage.getByTestId('page-thumb').locator('.scene-row'), 'a Usage Page draws its rows').toHaveCount(3);
  await usage.click();
  await expect(usage).toHaveAttribute('aria-current', 'true');
  await expect(page.getByTestId('dashboard-page')).toBeVisible();

  await importText(page, JSON.stringify({ ...dev, orientation: 'portrait' }), 'dev-portrait.json');
  await waitEvent(page, 'opened');
  await expect(page.getByTestId('deck-preview')).toHaveAttribute('data-orientation', 'portrait');
  await expect(thumb('shortcuts')).toHaveAttribute('data-orientation', 'portrait');
  expect(await ratio('shortcuts')).toBeCloseTo(600 / 1024, 1);
  await expect(thumb('shortcuts').locator('.thumb-tile')).toHaveCount(first.tiles.length);
});

test('Add Page adds a Page at the end of the column and shows it, and at 8 Pages it says why it can’t', async ({ page }) => {
  // The demo data toggle sits over the bottom of the column.
  await openLayout(page, '/?toggle=0');
  await page.getByTestId('start-starter').click();
  const add = page.getByTestId('page-add');
  await expect(add).toHaveText('Add Page');
  for (let n = 3; n <= 8; n++) {
    await add.click();
    await expect(page.getByTestId('page-entry')).toHaveCount(n);
    await expect(page.getByTestId('page-entry').last()).toHaveAttribute('aria-current', 'true');
    await expect(page.getByTestId('page-entry').last()).toBeInViewport();
  }
  await expect(pageTitles(page).last()).toHaveText('Page 8');
  await expect(add).toHaveAttribute('aria-disabled', 'true');
  await add.hover();
  await expect(page.locator('.tooltip')).toHaveText('A Layout holds up to 8 Pages');
  await add.click({ force: true });
  await expect(page.getByTestId('page-entry')).toHaveCount(8);
});

test('the Pages column folds away and back, the Device grows into the room, and this PC remembers it', async ({ page }) => {
  await importDev(page);
  const toggle = page.getByTestId('pages-toggle');
  const strip = page.getByTestId('page-strip');
  const width = async () => (await page.locator('.device').boundingBox())!.width;
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  await expect(toggle).toHaveAttribute('aria-controls', 'editor-pages');
  await expect(toggle).toHaveAccessibleName('Hide Pages');
  await expect(strip).toHaveAttribute('id', 'editor-pages');
  const head = await page.getByTestId('editor-head').boundingBox();
  const toggleBox = await toggle.boundingBox();
  const nameBox = await page.getByTestId('device-name').boundingBox();
  expect(toggleBox!.x, 'the button sits at the left of the header').toBeLessThan(head!.x + 24);
  expect(toggleBox!.x + toggleBox!.width, 'before the Device’s name').toBeLessThanOrEqual(nameBox!.x);
  expect((await strip.boundingBox())!.width).toBe(160);
  const motion = await page.getByTestId('editor').evaluate((el) => {
    const style = getComputedStyle(el);
    const root = getComputedStyle(document.documentElement);
    const list = (value: string) => value.split(/,\s*(?![^(]*\))/);
    const at = list(style.transitionProperty).indexOf('grid-template-columns');
    return { at, duration: list(style.transitionDuration)[at], easing: list(style.transitionTimingFunction)[at], expand: root.getPropertyValue('--duration-expand').trim() };
  });
  expect(motion.at, 'the column’s width animates').toBeGreaterThanOrEqual(0);
  expect(motion.duration).toBe(`${Number.parseFloat(motion.expand) / 1000}s`);
  expect(motion.easing).toBe('cubic-bezier(0.32, 0.72, 0, 1)');
  const open = await width();

  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await expect(toggle).toHaveAccessibleName('Show Pages');
  await expect(strip).toHaveAttribute('inert');
  await expect(strip).toBeHidden();
  await expect(page.getByRole('button', { name: 'Add Page' }), 'a folded column is hidden from screen readers').toHaveCount(0);
  await expect(page.getByRole('complementary', { name: 'Pages' })).toHaveCount(0);
  await expect.poll(width, { message: 'the Device grows into the room' }).toBeGreaterThan(open + 100);
  await toggle.focus();
  await page.keyboard.press('Shift+Tab');
  await expect(page.getByTestId('page-add'), 'a folded column is out of the tab order').not.toBeFocused();
  await expect(page.locator('.page-strip :focus')).toHaveCount(0);

  await page.reload();
  await page.getByRole('button', { name: 'Layout', exact: true }).click();
  await expect(page.getByTestId('pages-toggle'), 'this PC remembers the column folded').toHaveAttribute('aria-expanded', 'false');
  await expect(strip).toHaveAttribute('inert');
  await page.getByTestId('pages-toggle').click();
  await expect(strip).toBeVisible();
  await expect(strip).not.toHaveAttribute('inert');
  await expect.poll(width, { message: 'the Device gives the room back' }).toBe(open);
  await page.reload();
  await page.getByRole('button', { name: 'Layout', exact: true }).click();
  await expect(page.getByTestId('pages-toggle')).toHaveAttribute('aria-expanded', 'true');
});

test('in Preview, ← and → change Pages and the column follows', async ({ page }) => {
  await importDev(page);
  await pageEntry(page, 'shortcuts').click();
  await page.keyboard.press('p');
  const preview = page.getByTestId('deck-preview');
  await expect(preview).toHaveAttribute('data-mode', 'preview');
  await page.keyboard.press('ArrowRight');
  await expect(preview).toHaveAttribute('data-page', 'desktop');
  await expect(pageEntry(page, 'desktop')).toHaveAttribute('aria-current', 'true');
  await expect(pageEntry(page, 'shortcuts')).not.toHaveAttribute('aria-current');
  await expect(pageEntry(page, 'desktop'), 'focus in the column moves with the Page').toBeFocused();
  await page.keyboard.press('ArrowRight');
  await page.keyboard.press('ArrowRight');
  await expect(preview).toHaveAttribute('data-page', 'accounts');
  await expect(pageEntry(page, 'accounts')).toHaveAttribute('aria-current', 'true');
  await page.keyboard.press('ArrowLeft');
  await expect(pageEntry(page, 'usage')).toHaveAttribute('aria-current', 'true');
  await pageEntry(page, 'shortcuts').click();
  await expect(preview).toHaveAttribute('data-page', 'shortcuts');
});

test('an editor too narrow for both panels shows one at a time, and Details wins while it’s out', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 600 });
  await importDev(page);
  const strip = page.getByTestId('page-strip');
  const toggle = page.getByTestId('pages-toggle');
  const inspector = page.getByTestId('inspector');
  await expect(inspector).toBeVisible();
  await expect(strip).toHaveAttribute('data-open', 'false');
  await expect(toggle).toHaveAccessibleName('Show Pages');

  await page.getByTestId('add-tile').click();
  await toggle.click();
  await expect(strip).toBeVisible();
  await expect(inspector, 'showing Pages hides the Details panel').toBeHidden();
  await expect(page.getByTestId('add-tile'), 'and ends Add Tile').toHaveAttribute('aria-expanded', 'false');
  await pageEntry(page, 'desktop').click();
  await expect(page.getByTestId('deck-preview')).toHaveAttribute('data-page', 'desktop');

  await page.getByTestId('tile').first().click();
  await expect(inspector, 'picking a Tile brings Details back').toBeVisible();
  await expect(strip).toBeHidden();
  await page.getByTestId('details-toggle').click();
  await expect(strip, 'hiding Details brings the column back').toBeVisible();
  await expect(toggle).toHaveAccessibleName('Hide Pages');
});

declare global {
  interface Window {
    pushStates?: { state: string | null; at: number }[];
  }
}

test('Send to Device shows while there is something to send, turns to Sending… and Saved, then folds away until the next change', async ({ page }) => {
  const push = page.getByTestId('push');
  await page.getByTestId('start-pull').click();
  await waitEvent(page, 'pulled');
  await expect(push, 'a Layout just loaded has nothing to send').toHaveAttribute('data-state', 'hidden');
  await expect(push).toBeHidden();
  await expect(push).toHaveAttribute('inert', '');

  await tile(page, 'media0').click();
  await page.getByTestId('tile-clear').click();
  await expect(push).toHaveAttribute('data-state', 'send');
  await expect(push).toBeVisible();
  await expect(push).toHaveText('Send to Device');
  await expect(push).toHaveAttribute('aria-disabled', 'false');
  await page.evaluate(() => {
    const el = document.querySelector('[data-testid="push"]')!;
    const seen = [{ state: el.getAttribute('data-state'), at: performance.now() }];
    new MutationObserver(() => {
      const state = el.getAttribute('data-state');
      if (state !== seen.at(-1)?.state) seen.push({ state, at: performance.now() });
    }).observe(el, { attributes: true, attributeFilter: ['data-state'] });
    window.pushStates = seen;
  });
  await push.click();
  await expect(push).toHaveText('Saved');
  await waitEvent(page, 'pushed');
  await expect(push).toHaveAttribute('data-state', 'hidden');
  await expect(push).toBeHidden();
  const states = (await page.evaluate(() => window.pushStates))!;
  expect(states.map((s) => s.state)).toEqual(['send', 'sending', 'saved', 'hidden']);
  // Saved stays up for SAVED_MS (1600 ms in SendButton.tsx) before the button folds away.
  expect(states[3].at - states[2].at).toBeGreaterThanOrEqual(1500);

  await tile(page, 'media1').click();
  await page.getByTestId('tile-clear').click();
  await expect(push, 'the next change brings it back').toHaveAttribute('data-state', 'send');
  await expect(push).toBeVisible();
  await push.click();
  await waitEvent(page, 'pushed');
  await expect(push).toHaveAttribute('data-state', 'hidden');

  await page.getByTestId('layout-menu').click();
  await expect(page.getByTestId('menu-push')).toContainText('Send to Desk');
  await expect(page.getByTestId('menu-push')).toContainText('Send this Layout again, even without changes');
  await page.getByTestId('menu-push').click();
  await expect(await waitEvent(page, 'pushed')).toHaveText('Sent to Desk');
  await expect(push, 'a send from the menu shows Saved, then folds away').toHaveAttribute('data-state', /^(hidden|saved)$/);
  await expect(push).toHaveAttribute('data-state', 'hidden');
});

test("an editor error pops up a card, and its Details opens Settings at Errors with the error's detail", async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await importDev(page);
  const quiet = first.tiles.find((t) => t.id === 'quiet')!;
  await drag(page, 'f5', quiet.row, quiet.col, 'move');
  const taken = 'That spot is taken by “Mute”. Pick an empty one.';
  await expect(status(page).locator('.editor-status-text'), 'the status line keeps the words for screen readers').toHaveText(taken);
  await expect(status(page).locator('.editor-status-text')).toHaveClass(/\bvisually-hidden\b/);
  await expect(page.getByTestId('status-detail')).toHaveCount(0);
  const card = errorCard(page, taken);
  await expect(card).toBeVisible();
  await expect(card).toContainText('Layout · Desk');

  await importText(page, '{"schema": 1', 'broken.json');
  const broken = '“broken.json” isn\'t a Layout file. Nothing changed.';
  await expect(errorCard(page, broken)).toBeVisible();

  await errorCard(page, broken).getByRole('button', { name: 'Details' }).click();
  const settings = page.getByTestId('settings');
  await expect(settings).toHaveAttribute('data-section', 'errors');
  await expect(page.getByTestId('settings-title')).toHaveText('Errors');
  await expect(page.getByTestId('settings-nav-errors')).toHaveAttribute('aria-current', 'page');
  await expect(page.getByTestId('settings-nav-errors')).toContainText('2');
  await expect(page.getByRole('button', { name: 'Settings', exact: true })).toHaveAttribute('aria-current', 'page');
  const entries = page.getByTestId('error-entry');
  await expect(entries).toHaveCount(2);
  await expect(entries.nth(0), 'newest first').toContainText(broken);
  await expect(entries.nth(0)).toContainText('Layout · Desk');
  await expect(entries.nth(0).getByTestId('error-detail')).not.toBeEmpty();
  await expect(entries.nth(1)).toContainText(taken);
  await expect(entries.nth(1).getByTestId('error-detail')).toHaveText('overlaps quiet');

  await entries.nth(1).getByTestId('error-copy').click();
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toContain('overlaps quiet');
  const copied = await page.evaluate(() => navigator.clipboard.readText());
  expect(copied).toContain(taken);
  expect(copied).toContain('Layout · Desk');

  await page.getByTestId('errors-clear').click();
  await expect(entries).toHaveCount(0);
  await expect(page.getByTestId('errors-empty')).toHaveText('No errors.');
  await expect(page.getByTestId('settings-nav-errors')).not.toContainText(/\d/);
});
