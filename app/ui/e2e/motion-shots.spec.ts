import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { cellPoint, importText, openLayout, placeOf, swipe, tile, waitEvent } from './layout-helpers.ts';

const folder = process.env.TD_SHOTS ?? '';
const devText = readFileSync(join(import.meta.dirname, '..', '..', '..', 'schema', 'examples', 'dev.json'), 'utf8');
const AT_VISIBLE_ICON = { position: { x: 22, y: 22 } };
const nav = (page: Page) => page.getByRole('navigation', { name: 'Main' });
const navButton = (page: Page, name: string) => nav(page).getByRole('button', { name, exact: true });

test.skip(!folder, 'set TD_SHOTS to a folder to film each animation');

async function shoot(page: Page, name: string) {
  await page.screenshot({ path: join(folder, `${name}.png`) });
}

async function film(page: Page, name: string, act: () => Promise<void>) {
  await page.evaluate(() => document.fonts.ready);
  await shoot(page, `${name}-1-before`);
  await act();
  await page.waitForFunction(() => document.getAnimations().some((a) => a.playState === 'running'));
  await page.evaluate(() => document.getAnimations().forEach((a) => a.pause()));
  for (const [step, at] of [[2, 1 / 3], [3, 2 / 3]] as const) {
    await page.evaluate((at) => {
      for (const a of document.getAnimations()) a.currentTime = Number(a.effect?.getComputedTiming().endTime ?? 0) * at;
    }, at);
    await shoot(page, `${name}-${step}-${step === 2 ? 'third' : 'two-thirds'}`);
  }
  await page.evaluate(() => document.getAnimations().forEach((a) => a.finish()));
  await page.waitForTimeout(100);
  await shoot(page, `${name}-4-after`);
}

const demo = (page: Page) => ({
  offline: (id: string) => page.evaluate((id) => window.touchdeckDemo?.offline(id), id),
  connect: (id: string) =>
    page.evaluate((id) => window.touchdeckDemo?.device(id, { link: { state: 'connected', port: 'COM4', since_ms: 0, protocol: 2 } }), id),
});

async function home(page: Page, query = '') {
  await page.goto(`/${query}`);
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
}

test('01 the Update icon opens', async ({ page }) => {
  await home(page, '?update=available');
  await film(page, '01-update-icon-open', () => page.locator('.update-button').hover(AT_VISIBLE_ICON));
});

test('02 Send to Device slides open for a change', async ({ page }) => {
  await openLayout(page);
  await page.getByTestId('start-pull').click();
  await waitEvent(page, 'pulled');
  await tile(page, 'media0').click();
  await page.waitForTimeout(500);
  await film(page, '02-send-open', () => page.getByTestId('tile-clear').click());
});

test('03 an error card pops up', async ({ page }) => {
  await openLayout(page);
  await film(page, '03-error-card', () => importText(page, '{"schema": 1', 'broken.json'));
});

test('04 a tooltip appears', async ({ page }) => {
  await home(page);
  await film(page, '04-tooltip', () => navButton(page, 'Home').hover());
});

test('05 a Device on Home lifts under the pointer', async ({ page }) => {
  await home(page, '?devices=1');
  await film(page, '05-device-lift', () => page.locator('.device-hit').hover());
});

test('06 a Device on Home is pressed', async ({ page }) => {
  await home(page, '?devices=1');
  await page.locator('.device-hit').hover();
  await page.waitForTimeout(300);
  await film(page, '06-device-press', () => page.mouse.down());
  await page.mouse.move(600, 20);
  await page.mouse.up();
});

for (const theme of ['light', 'dark'] as const) {
  test(`07 a Device's screen lights up as it connects, ${theme}`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: theme });
    await home(page, '?devices=1');
    await demo(page).offline('mock0001');
    await expect(page.locator('.device-scene')).toHaveCSS('opacity', '0.7');
    await film(page, `07-device-lights-${theme}`, () => demo(page).connect('mock0001'));
  });
}

test('08 a switch is turned on', async ({ page }) => {
  await openLayout(page);
  await importText(page, devText, 'dev.json');
  await waitEvent(page, 'opened');
  await page.keyboard.press('p');
  await film(page, '08-switch', () => page.getByTestId('actual-size').click());
});

test('09 a button is pressed', async ({ page }) => {
  await home(page);
  await navButton(page, 'Layout').hover();
  await page.waitForTimeout(700);
  await film(page, '09-button-press', () => page.mouse.down());
  await page.mouse.move(600, 300);
  await page.mouse.up();
});

test('10 a dropped Tile settles into its cell', async ({ page }) => {
  await openLayout(page);
  await importText(page, devText, 'dev.json');
  await waitEvent(page, 'opened');
  const at = await placeOf(page, 'f5');
  const grid = page.getByTestId('grid');
  const from = await cellPoint(page, at.row, at.col);
  const to = await cellPoint(page, Number(await grid.getAttribute('data-rows')) - 1, Number(await grid.getAttribute('data-cols')) - 1);
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(to.x + 30, to.y + 20, { steps: 8 });
  await film(page, '10-tile-settle', () => page.mouse.up());
});

test('13 the try-it Page slide', async ({ page }) => {
  await openLayout(page);
  await importText(page, devText, 'dev.json');
  await waitEvent(page, 'opened');
  await page.keyboard.press('p');
  const preview = page.getByTestId('deck-preview');
  await page.evaluate(() => document.fonts.ready);
  await shoot(page, '13-page-slide-1-before');
  await tile(page, 'accounts_jump').click();
  await shoot(page, '13-page-slide-2-during');
  await expect(preview).not.toHaveAttribute('data-sliding');
  await shoot(page, '13-page-slide-3-after');
  await swipe(page, 200);
  await expect(preview).not.toHaveAttribute('data-sliding');
});
