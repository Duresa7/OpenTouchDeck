import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { consoleErrors, sideScroll, spills } from './break-helpers.ts';
import { openDeviceSettings, openSettings, pageEntry, showPages } from './layout-helpers.ts';

const survey = process.env.TD_BREAK_SURVEY;
const shots = process.env.TD_SHOTS;

// A Preset name is clamped to two lines, with the whole name in a tooltip. The Update icon is laid out at full width
// and clipped to its icon, so its label runs out of the nav rail over the screen.
const CUT_BY_DESIGN = ['.preset-name', '.nav-rail:has(> .update-slot)', '.update-slot'];

const SIZES = [
  { size: 'min', viewport: { width: 800, height: 600 } },
  { size: 'default', viewport: { width: 1180, height: 760 } },
  { size: '4k', viewport: { width: 3840, height: 2160 } },
] as const;

type Size = (typeof SIZES)[number]['size'];
type Theme = 'light' | 'dark';

interface At {
  theme: Theme;
  size: Size;
}

async function settle(page: Page) {
  await page.evaluate(() => document.fonts.ready);
  await page.waitForFunction(() => document.getAnimations().every((animation) => animation.playState !== 'running' || animation.effect?.getTiming().iterations === Infinity));
}

async function check(page: Page, { name, theme, size, fullPage = true }: At & { name: string; fullPage?: boolean }) {
  await settle(page);
  const key = `${name}-${size}-${theme}`;
  const result = { scroll: await sideScroll(page), spills: await spills(page, CUT_BY_DESIGN) };
  if (survey) {
    mkdirSync(survey, { recursive: true });
    await page.screenshot({ path: join(survey, `${key}.png`), fullPage });
    writeFileSync(join(survey, `${key}.json`), `${JSON.stringify(result, null, 2)}\n`);
    return;
  }
  expect(result, key).toEqual({ scroll: 0, spills: [] });
}

async function shot(target: Locator | Page, { row, theme, size }: At & { row: string }) {
  if (!shots) return;
  mkdirSync(shots, { recursive: true });
  await target.screenshot({ path: join(shots, `${row}-${size}-${theme}.png`) });
}

async function boxOf(target: Locator) {
  const box = await target.boundingBox();
  if (!box) throw new Error(`${target.toString()} isn't drawn`);
  return box;
}

const uncovered = (target: Locator) =>
  target.evaluate((el) => {
    const box = el.getBoundingClientRect();
    const hit = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
    return hit !== null && el.contains(hit);
  });

const grouped = (page: Page, value: number) => page.evaluate((n) => new Intl.NumberFormat().format(n), value);

const nav = (page: Page) => page.getByRole('navigation', { name: 'Main' });

async function home(page: Page, query: string) {
  await page.goto(`/?toggle=0&data=worst${query}`);
  await expect(page.locator('main.home')).toBeVisible();
}

async function editor(page: Page, device: string) {
  await page.locator(`.device-figure:has(.device-hit[aria-label="Edit ${device}"]) .device-hit`).click();
  await expect(page.getByTestId('editor')).toBeVisible();
}

/** Shows a Page from the Pages column, bringing the column out first when a narrow editor hides it. */
async function choosePage(page: Page, id: string) {
  await showPages(page);
  await pageEntry(page, id).click();
}
const tile = (page: Page, id: string) => page.locator(`[data-testid="tile"][data-id="${id}"]`);

const LONGEST_NAME = 'StreamingDeskLeftOfTheMonitorArmForSaturdayShows';

for (const theme of ['light', 'dark'] as const) {
  for (const { size, viewport } of SIZES) {
    const at: At = { theme, size };
    test.describe(`${theme} at ${size}`, () => {
      test.use({ colorScheme: theme, viewport });

      test('home with ten, one and no Devices, and every one offline', async ({ page }) => {
        const errors = consoleErrors(page);
        await home(page, '');
        await expect(page.locator('.device-row > .device-figure')).toHaveCount(11);
        await expect(page.locator('link[rel="icon"]')).toHaveCount(1);
        await check(page, { ...at, name: 'home-ten' });
        await home(page, '&devices=1');
        const preview = page.locator('.device-scene').first();
        expect(await preview.locator('.preview-box').count(), 'letters the Device lacks are empty boxes').toBeGreaterThan(0);
        expect(await preview.locator('svg').count(), 'each Tile clips what it draws').toBeGreaterThan(0);
        await check(page, { ...at, name: 'home-one' });
        await shot(page.locator('.device-screen').first(), { ...at, row: 'row-13-21-home-preview' });
        await home(page, '&devices=0');
        await check(page, { ...at, name: 'home-none' });
        await home(page, '&offline=all');
        await check(page, { ...at, name: 'home-offline' });
        expect(errors).toEqual([]);
      });

      test('the editor with the full Layout, every Page and the inspector', async ({ page }) => {
        const errors = consoleErrors(page);
        await home(page, '');
        await nav(page).getByRole('button', { name: 'Layout', exact: true }).click();
        await expect(page.getByTestId('editor')).toBeVisible();
        await page.getByTestId('start-pull').click();
        await expect(page.getByTestId('grid').getByTestId('tile')).toHaveCount(64);
        await check(page, { ...at, name: 'editor-full' });

        const head = page.locator('.editor-head');
        const push = page.getByTestId('push');
        await expect(push, 'a Layout just loaded has nothing to send').toHaveAttribute('data-state', 'hidden');
        // Send to Device shows once there is a change to send.
        const layoutName = page.getByTestId('layout-name');
        await layoutName.fill('Edited Layout');
        await layoutName.press('Enter');
        await expect(push).toHaveAttribute('data-state', 'send');
        const headBox = await boxOf(head);
        const pushBox = await boxOf(push);
        expect(pushBox.x + pushBox.width, 'Send to Device stays inside the header').toBeLessThanOrEqual(headBox.x + headBox.width);
        expect(await uncovered(push), 'Send to Device can be clicked').toBe(true);
        await expect(page.getByTestId('device-name')).toHaveAttribute('title', 'Aleksandra Wiśniewska-Kowalczyk’s desk');
        await expect(page.getByTestId('connection')).toHaveText(/: Connected over USB$/);
        await shot(head, { ...at, row: 'row-01-14-editor-header' });

        const canvas = page.locator('.canvas');
        const canvasBox = await boxOf(canvas);
        await page.mouse.move(canvasBox.x + canvasBox.width / 2, canvasBox.y + 8);
        await page.mouse.wheel(0, 4000);
        await expect.poll(() => canvas.evaluate((el) => el.scrollTop + el.clientHeight >= el.scrollHeight - 1), { message: 'the canvas scrolls to its foot' }).toBe(true);
        const foot = await boxOf(page.locator('.canvas-foot'));
        const rail = await boxOf(page.locator('.nav-rail'));
        expect(foot.x, 'the canvas foot clears the nav').toBeGreaterThanOrEqual(rail.x + rail.width);
        await shot(canvas, { ...at, row: 'row-02-editor-canvas-foot' });
        await canvas.evaluate((el) => el.scrollTo(0, 0));

        await showPages(page);
        const add = page.getByTestId('page-add');
        await add.scrollIntoViewIfNeeded();
        expect(await uncovered(add), 'Add Page, at the end of the Pages column, clears the nav').toBe(true);
        await expect(pageEntry(page, 'full')).toHaveAttribute('title', 'Wiśniewska-Kowalczyk Pro');
        await check(page, { ...at, name: 'editor-pages' });

        await tile(page, 't6').click();
        const warning = page.getByTestId('tile-fields').locator('.field-hint.is-warning');
        await expect(warning).toContainText("can't show 👩‍👩‍👧‍👦 and draws an empty box instead.");
        await check(page, { ...at, name: 'editor-tile' });
        await shot(warning, { ...at, row: 'row-10-glyph-warning' });
        for (const id of ['live', 'rtl', 'cjk', 'usage', 'empty', 'jump', 'one']) {
          await choosePage(page, id);
          await check(page, { ...at, name: `editor-page-${id}` });
        }
        await choosePage(page, 'live');
        for (const id of ['vol', 'np', 'cpu', 'gif', 'gone']) {
          await tile(page, id).click();
          await check(page, { ...at, name: `editor-live-${id}` });
        }
        await choosePage(page, 'jump');
        await tile(page, 'j1').click();
        await check(page, { ...at, name: 'editor-launch' });
        await tile(page, 'j2').click();
        await check(page, { ...at, name: 'editor-open' });
        await page.getByTestId('add-tile').click();
        await expect(page.getByTestId('add-where')).toHaveText('The J Page is full. A Layout holds up to 8 Pages.');
        await check(page, { ...at, name: 'editor-add' });
        await shot(page.locator('.inspector-head'), { ...at, row: 'row-05-add-tile-head' });
        const labels = page.getByTestId('add-presets').locator('[data-testid="preset-group"][data-id="custom"] .preset-name');
        await expect(labels.filter({ hasText: /^Kitchen tv$/ }).first()).toBeVisible();
        const cut = await labels.evaluateAll((all) => all.filter((label) => /\u200d$/u.test(label.textContent ?? '')).length);
        expect(cut, 'no label ends inside an emoji').toBe(0);
        const arabic = labels.filter({ hasText: 'درجة' }).first();
        expect(await arabic.evaluate((el) => getComputedStyle(el).direction)).toBe('rtl');
        await shot(page.locator('[data-testid="preset-group"][data-id="custom"] .preset-grid'), { ...at, row: 'row-11-12-preset-labels' });
        await page.getByTestId('add-tile').click();
        await page.getByTestId('preview').click();
        await check(page, { ...at, name: 'editor-try-it' });
        await page.getByTestId('preview').click();
        await page.getByTestId('device-rename').click();
        await expect(page.getByTestId('device-rename-input')).toBeFocused();
        await check(page, { ...at, name: 'editor-rename' });
        await page.keyboard.press('Escape');
        await openDeviceSettings(page);
        await check(page, { ...at, name: 'device-settings' });
        expect(errors).toEqual([]);
      });

      test('the editor for a Device with no Layout and for one with old Firmware', async ({ page }) => {
        const errors = consoleErrors(page);
        await home(page, '');
        await editor(page, LONGEST_NAME);
        await expect(page.getByTestId('start-layout')).toBeVisible();
        await check(page, { ...at, name: 'editor-start' });
        await shot(page.locator('.start-choices'), { ...at, row: 'row-23-start-choices' });
        await home(page, '');
        await editor(page, 'J');
        await check(page, { ...at, name: 'editor-offline' });
        await home(page, '');
        await editor(page, 'مكتب الاستوديو الرئيسي');
        await check(page, { ...at, name: 'editor-rtl-name' });
        expect(errors).toEqual([]);
      });

      test('integrations, each detail and the accounts', async ({ page }) => {
        const errors = consoleErrors(page);
        await home(page, '');
        await nav(page).getByRole('button', { name: 'Integrations' }).click();
        await expect(page.getByTestId('integration-list')).toBeVisible();
        await expect(page.locator('[data-testid="integration"][data-id="this_pc"] .integration-glyph svg')).toHaveClass(/lucide-monitor/);
        await expect(page.locator('[data-testid="integration"][data-id="custom"] .integration-glyph svg')).toHaveClass(/lucide-braces/);
        const switches = await page.getByTestId('integration-switch').evaluateAll((all) => all.map((el) => Math.round(el.getBoundingClientRect().right)));
        expect(new Set(switches).size, 'every switch lines up on the right').toBe(1);
        await check(page, { ...at, name: 'integrations' });
        await shot(page.getByTestId('integration-list'), { ...at, row: 'row-07-08-integration-list' });
        for (const id of ['volume', 'media', 'this_pc']) await expect(page.locator(`[data-testid="integration"][data-id="${id}"] [data-testid="integration-settings"]`)).toHaveCount(0);
        for (const id of ['cliproxyapi', 'custom', 'icon-packs', 'brandfetch']) {
          await page.locator(`[data-testid="integration"][data-id="${id}"] [data-testid="integration-settings"]`).click();
          await expect(page.getByTestId('integration-detail')).toHaveAttribute('data-id', id);
          await check(page, { ...at, name: `integration-${id}` });
          if (id === 'cliproxyapi') {
            const glyph = await boxOf(page.locator('.detail-head .integration-glyph'));
            const title = await boxOf(page.locator('.detail-title'));
            const toggle = await boxOf(page.getByTestId('detail-switch'));
            expect(Math.abs(glyph.y - title.y), 'the glyph sits beside the name').toBeLessThanOrEqual(8);
            expect(Math.abs(toggle.y + toggle.height / 2 - (title.y + title.height / 2)), 'the switch sits on the name').toBeLessThanOrEqual(4);
            expect(toggle.x, "the switch doesn't overlap the name").toBeGreaterThanOrEqual(title.x + title.width);
            await shot(page.locator('.detail-head'), { ...at, row: 'row-09-detail-head' });
            const table = page.getByTestId('usage-accounts');
            await table.scrollIntoViewIfNeeded();
            expect(await page.locator('.usage-scroll').evaluate((el) => el.scrollWidth - el.clientWidth), 'the accounts fit their sheet').toBeLessThanOrEqual(1);
            expect((await boxOf(table.locator('thead th').first())).width, 'the Account column keeps its room').toBeGreaterThanOrEqual(159);
            await expect(table).toContainText(`${await grouped(page, 1_284_567_890)} OK`);
            await expect(table).toContainText(`${await grouped(page, 999_999_999)}%`);
            await expect(table).toContainText('Quota exhausted temporarily');
            const meters = await table.locator('.meter').evaluateAll((all) => all.map((meter) => meter.getBoundingClientRect().height));
            expect(meters.length > 0 && meters.every((height) => height > 0), 'every limit draws its meter').toBe(true);
            await shot(table, { ...at, row: 'row-03-06-20-25-accounts' });
          }
          if (id === 'custom') {
            const sources = page.locator('[data-testid="config-field"][data-field="sources"]');
            expect((await boxOf(sources)).height, 'the sources JSON shows a dozen lines').toBeGreaterThanOrEqual(200);
            await shot(sources, { ...at, row: 'row-15-sources-json' });
          }
          await page.getByTestId('detail-back').click();
        }
        await page.evaluate(() => window.touchdeckDemo?.setHub(false));
        await page.locator('[data-testid="integration"][data-id="cliproxyapi"] [data-testid="integration-settings"]').click();
        await check(page, { ...at, name: 'integration-hub-down' });
        expect(errors).toEqual([]);
      });

      test('settings, paired browsers and troubleshooting', async ({ page }) => {
        const errors = consoleErrors(page);
        await home(page, '');
        await openSettings(page);
        await check(page, { ...at, name: 'settings', fullPage: false });
        await openSettings(page, 'browsers');
        await check(page, { ...at, name: 'settings-browsers', fullPage: false });
        await openSettings(page, 'troubleshooting');
        await check(page, { ...at, name: 'troubleshooting', fullPage: false });
        const picker = page.locator('.settings-content .row:has(> .segmented)').first();
        await picker.scrollIntoViewIfNeeded();
        await check(page, { ...at, name: 'troubleshooting-console', fullPage: false });
        await shot(picker, { ...at, row: 'row-24-console-device-picker' });
        expect(errors).toEqual([]);
      });

      test('device setup for a new Device', async ({ page }) => {
        const errors = consoleErrors(page);
        await home(page, '&devices=3');
        await page.evaluate(() => window.touchdeckDemo?.fresh('mock0099'));
        const setup = page.getByTestId('onboarding');
        await expect(setup).toBeVisible();
        await check(page, { ...at, name: 'setup-start', fullPage: false });
        await page.getByTestId('onboarding-name').fill(LONGEST_NAME);
        await check(page, { ...at, name: 'setup-name', fullPage: false });
        const next = page.getByTestId('onboarding-next');
        await setup.hover();
        await page.mouse.wheel(0, 2000);
        await expect.poll(() => uncovered(next), { message: 'Next can be scrolled to and clicked' }).toBe(true);
        await shot(page, { ...at, row: 'row-22-26-setup' });
        await next.click();
        await check(page, { ...at, name: 'setup-next', fullPage: false });
        expect(errors).toEqual([]);
      });

      test('the Update icon and dialog with the longest release', async ({ page }) => {
        const errors = consoleErrors(page);
        await home(page, '&update=available');
        const icon = nav(page).getByRole('button', { name: 'Update available' });
        await expect(icon).toBeVisible();
        await icon.hover({ position: { x: 22, y: 22 } });
        await check(page, { ...at, name: 'update-icon' });
        await icon.click();
        const dialog = page.getByRole('dialog');
        await expect(dialog).toBeVisible();
        await check(page, { ...at, name: 'update-dialog', fullPage: false });
        for (const name of ['Later', 'Update']) {
          const action = dialog.getByRole('button', { name, exact: true });
          await expect(action, `${name} stays in view`).toBeInViewport({ ratio: 1 });
          expect(await uncovered(action)).toBe(true);
        }
        await shot(page, { ...at, row: 'row-04-update-dialog' });
        expect(errors).toEqual([]);
      });
    });
  }
}
