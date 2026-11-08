import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { cellPoint, importText, openLayout, placeOf, swipe, tile, waitEvent } from './layout-helpers.ts';
import { droppedFrames, expectArrived, expectHeld, expectJumped, expectTurnedBack, frames, longestGap, slowMotion, trace } from './motion-helpers.ts';

const nav = (page: Page) => page.getByRole('navigation', { name: 'Main' });
const navButton = (page: Page, name: string) => nav(page).getByRole('button', { name, exact: true });
const settings = (page: Page) => page.getByRole('dialog', { name: 'Settings' });
const AT_VISIBLE_ICON = { position: { x: 22, y: 22 } };
const devText = readFileSync(join(import.meta.dirname, '..', '..', '..', 'schema', 'examples', 'dev.json'), 'utf8');
const AWAY = { x: 600, y: 300 };
const JANK_MS = 100;

async function widthOf(locator: Locator) {
  const box = await locator.boundingBox();
  if (!box) throw new Error('The element is not on screen.');
  return box.width;
}

async function home(page: Page, query = '') {
  await page.goto(`/${query}`);
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}

const demo = (page: Page) => ({
  offline: (id: string) => page.evaluate((id) => window.touchdeckDemo?.offline(id), id),
  connect: (id: string) =>
    page.evaluate((id) => window.touchdeckDemo?.device(id, { link: { state: 'connected', port: 'COM4', since_ms: 0, protocol: 2 } }), id),
  updates: (holdAt: number) => page.evaluate((holdAt) => window.touchdeckDemo?.updates.script({ holdAt }), holdAt),
});

async function tryIt(page: Page) {
  await openLayout(page);
  await importText(page, devText, 'dev.json');
  await waitEvent(page, 'opened');
  await page.keyboard.press('p');
  await expect(page.getByTestId('deck-preview')).toHaveAttribute('data-mode', 'preview');
}

test.describe('with motion', () => {
  test('the Update icon closes from where it is when the pointer leaves mid-open, and reopens from there', async ({ page }) => {
    await home(page, '?update=available');
    await slowMotion(page);
    const icon = page.locator('.update-button');
    const opening = await trace(page, [{ selector: '.update-button', reading: 'visible-width' }]);
    await icon.hover(AT_VISIBLE_ICON);
    await page.waitForTimeout(400);
    await page.mouse.move(AWAY.x, AWAY.y);
    await expect(page.locator('.update-slot')).not.toHaveClass(/is-expanded/);
    await page.waitForTimeout(2500);
    const [opened] = await opening.stop();
    await icon.hover(AT_VISIBLE_ICON);
    await page.waitForTimeout(3500);
    const open = (await widthOf(icon)) + 4;
    expectTurnedBack(opened, { rest: 44, goal: open });
    await expect(page.locator('.update-label')).toHaveCSS('opacity', '1');

    const closing = await trace(page, [{ selector: '.update-button', reading: 'visible-width' }]);
    await page.mouse.move(AWAY.x, AWAY.y);
    await page.waitForTimeout(400);
    await icon.hover(AT_VISIBLE_ICON);
    await page.waitForTimeout(3500);
    const [closed] = await closing.stop();
    expectTurnedBack(closed, { rest: open, goal: 44 });
  });

  test('Escape mid-open shrinks Settings away from where it is, then it opens fully', async ({ page }) => {
    await home(page);
    await slowMotion(page);
    const leaving = await trace(page, [{ selector: '.dialog-settings', reading: 'scale' }, { selector: '.dialog-settings', reading: 'opacity' }]);
    await navButton(page, 'Settings').click();
    await page.waitForTimeout(400);
    await page.keyboard.press('Escape');
    await expect(settings(page)).toHaveCount(0, { timeout: 5000 });
    const [scale, opacity] = await leaving.stop();
    expectTurnedBack(scale, { rest: 0.97, goal: 1, removed: true });
    expectTurnedBack(opacity, { rest: 0, goal: 1, removed: true });
    await expect(navButton(page, 'Settings')).toBeFocused();

    const entering = await trace(page, [{ selector: '.dialog-settings', reading: 'scale' }]);
    await navButton(page, 'Settings').click();
    await expect(settings(page)).toHaveCSS('scale', 'none', { timeout: 5000 });
    await expect(settings(page)).toHaveCSS('opacity', '1');
    const [grown] = await entering.stop();
    expectArrived(grown, { rest: 0.97, goal: 1 });
  });

  test('a second click mid-open closes the Device panel from where it is, and its chevron turns back', async ({ page }) => {
    await openLayout(page);
    await slowMotion(page);
    const name = page.getByTestId('device-name');
    const panel = page.getByTestId('device-panel');
    const toggled = await trace(page, [
      { selector: '[data-testid="device-panel"]', reading: 'scale' },
      { selector: '[data-testid="device-name"] .chev', reading: 'rotate' },
    ]);
    await name.click();
    await page.waitForTimeout(350);
    await name.click();
    await expect(panel).toHaveCount(0, { timeout: 5000 });
    await page.waitForTimeout(500);
    const [scale, rotate] = await toggled.stop();
    expectTurnedBack(scale, { rest: 0.97, goal: 1, removed: true });
    expectTurnedBack(rotate, { rest: 0, goal: 180 });

    await name.click();
    await expect(panel).toHaveCSS('scale', 'none', { timeout: 5000 });
    await expect(panel).toHaveCSS('opacity', '1');
    await expect(name.locator('.chev')).toHaveCSS('rotate', '180deg', { timeout: 5000 });
  });

  test('a tooltip fades back from where it is when the pointer leaves mid-open', async ({ page }) => {
    await home(page);
    await slowMotion(page);
    const shown = await trace(page, [{ selector: '.tooltip', reading: 'matrix-a' }]);
    await navButton(page, 'Home').hover();
    await expect(page.locator('.tooltip')).toHaveCount(1);
    await page.waitForTimeout(350);
    await page.mouse.move(AWAY.x, AWAY.y);
    await expect(page.locator('.tooltip')).toHaveCount(0, { timeout: 5000 });
    const [scale] = await shown.stop();
    expectTurnedBack(scale, { rest: 0.96, goal: 1, removed: true });

    await navButton(page, 'Layout').hover();
    await expect(page.locator('.tooltip')).toHaveText('Layout');
    await expect(page.locator('.tooltip')).toHaveCSS('opacity', '1', { timeout: 5000 });
    await expect(page.locator('.tooltip')).toHaveCSS('transform', 'none');
  });

  test('a Device on Home lifts by its transform and a shadow crossfade, and settles back from where it is', async ({ page }) => {
    await home(page, '?devices=1');
    const bezel = page.locator('.device-bezel');
    await expect(bezel).toHaveCSS('transition-property', 'transform');
    expect(await bezel.evaluate((el) => [getComputedStyle(el, '::before').transitionProperty, getComputedStyle(el, '::after').transitionProperty])).toEqual(['opacity', 'opacity']);
    const resting = await bezel.evaluate((el) => getComputedStyle(el).boxShadow);
    await slowMotion(page);
    const lifted = await trace(page, [
      { selector: '.device-bezel', reading: 'matrix-y' },
      { selector: '.device-bezel', reading: 'opacity', pseudo: '::after' },
    ]);
    await page.locator('.device-hit').hover();
    await page.waitForTimeout(300);
    await page.mouse.move(AWAY.x, 20);
    await page.waitForTimeout(2500);
    const [lift, shadow] = await lifted.stop();
    expectTurnedBack(lift, { rest: 0, goal: -2 });
    expectTurnedBack(shadow, { rest: 0, goal: 1 });

    await page.locator('.device-hit').hover();
    await expect(bezel).toHaveCSS('transform', 'matrix(1, 0, 0, 1, 0, -2)', { timeout: 5000 });
    expect(await bezel.evaluate((el) => [getComputedStyle(el, '::before').opacity, getComputedStyle(el, '::after').opacity])).toEqual(['0', '1']);
    expect(await bezel.evaluate((el) => getComputedStyle(el).boxShadow)).toBe(resting);
  });

  test('pressing a Device on Home scales it to 0.985, and letting go turns it back from where it is', async ({ page }) => {
    await home(page, '?devices=1');
    const hit = page.locator('.device-hit');
    await hit.hover();
    await page.mouse.down();
    await expect(hit).toHaveCSS('transform', 'matrix(0.985, 0, 0, 0.985, 0, 0)');
    await page.mouse.move(AWAY.x, 20);
    await page.mouse.up();
    await expect(hit).toHaveCSS('transform', 'none');
    await expect(page.locator('.device-row')).toBeVisible();

    await slowMotion(page);
    const pressed = await trace(page, [{ selector: '.device-hit', reading: 'matrix-a' }]);
    await hit.hover();
    await page.mouse.down();
    await page.waitForTimeout(300);
    await page.mouse.move(AWAY.x, 20);
    await page.mouse.up();
    await page.waitForTimeout(2000);
    const [scale] = await pressed.stop();
    expectTurnedBack(scale, { rest: 1, goal: 0.985 });
  });

  test("a Device's screen lights up as it connects and dims as it goes, turning back from where it is", async ({ page }) => {
    await page.emulateMedia({ colorScheme: 'dark' });
    await home(page, '?devices=1');
    const scene = page.locator('.device-scene');
    const spill = page.locator('.device-spill');
    await demo(page).offline('mock0001');
    await expect(scene).toHaveCSS('opacity', '0.7');
    await expect(spill).toHaveCSS('opacity', '0');

    await slowMotion(page);
    const lit = await trace(page, [
      { selector: '.device-scene', reading: 'opacity' },
      { selector: '.device-spill', reading: 'opacity' },
    ]);
    await demo(page).connect('mock0001');
    await expect(page.getByRole('img', { name: /^Desk: Connected/ })).toBeVisible();
    await page.waitForTimeout(400);
    await demo(page).offline('mock0001');
    await page.waitForTimeout(2500);
    const [screen, light] = await lit.stop();
    expectTurnedBack(screen, { rest: 0.7, goal: 1 });
    expectTurnedBack(light, { rest: 0, goal: 0.32 });

    await demo(page).connect('mock0001');
    await expect(scene).toHaveCSS('opacity', '1', { timeout: 5000 });
    await expect(spill).toHaveCSS('opacity', '0.32', { timeout: 5000 });
  });

  test('a switch thumb slides back from where it is when clicked again mid-slide', async ({ page }) => {
    await tryIt(page);
    await slowMotion(page);
    const toggle = page.getByTestId('actual-size');
    const slid = await trace(page, [{ selector: '[data-testid="actual-size"] .switch-thumb', reading: 'translate-x' }]);
    await toggle.click();
    await page.waitForTimeout(350);
    await toggle.click();
    await page.waitForTimeout(2500);
    const [thumb] = await slid.stop();
    expectTurnedBack(thumb, { rest: 0, goal: 14 });
    await expect(toggle).not.toHaveAttribute('data-checked');
  });

  test('pressing a button scales it to 0.97 and letting go turns it back', async ({ page }) => {
    await home(page);
    const button = navButton(page, 'Layout');
    await button.hover();
    await page.mouse.down();
    await expect(button).toHaveCSS('transform', 'matrix(0.97, 0, 0, 0.97, 0, 0)');
    await page.mouse.move(AWAY.x, AWAY.y);
    await page.mouse.up();
    await expect(button).toHaveCSS('transform', 'none');
  });

  test('a dropped Tile settles into its cell from where it was let go', async ({ page }) => {
    await openLayout(page);
    await importText(page, devText, 'dev.json');
    await waitEvent(page, 'opened');
    const at = await placeOf(page, 'f5');
    const grid = page.getByTestId('grid');
    const free = { row: Number(await grid.getAttribute('data-rows')) - 1, col: Number(await grid.getAttribute('data-cols')) - 1 };
    const from = await cellPoint(page, at.row, at.col);
    const to = await cellPoint(page, free.row, free.col);
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    await page.mouse.move(to.x + 14, to.y + 10, { steps: 8 });
    await slowMotion(page);
    const settled = await trace(page, [{ selector: '[data-testid="tile"][data-id="f5"]', reading: 'matrix-x' }]);
    await page.mouse.up();
    await page.waitForTimeout(2500);
    const [x] = await settled.stop();
    expect(await placeOf(page, 'f5')).toMatchObject(free);
    const dropped = x.findIndex((v) => v > 1);
    expect(dropped).toBeGreaterThanOrEqual(0);
    expectArrived(x.slice(dropped), { rest: x[dropped], goal: 0 });
    await expect(tile(page, 'f5')).toHaveCSS('transform', 'none');
  });

  test('download progress fills to where the download is', async ({ page }) => {
    await home(page, '?update=available');
    await demo(page).updates(0.3);
    await page.locator('.update-button').click(AT_VISIBLE_ICON);
    await page.getByRole('dialog').getByRole('button', { name: 'Update' }).click();
    const fill = page.locator('.update-progress span');
    await expect(fill).toHaveCSS('transition-duration', '0.12s');
    await expect(fill).toHaveCSS('transform', 'matrix(0.3, 0, 0, 1, 0, 0)');
  });

  test('the try-it Page slide ends on the new Page, and a press mid-slide stops it on that Page as the Device does', async ({ page }) => {
    await tryIt(page);
    const preview = page.getByTestId('deck-preview');
    const sliding = await trace(page, [{ selector: '[data-testid="deck-preview"] .scene-track', reading: 'matrix-x' }]);
    await swipe(page, -200);
    await expect(preview).toHaveAttribute('data-page', 'desktop');
    await expect(preview).not.toHaveAttribute('data-sliding');
    const [x] = await sliding.stop();
    expect(x.length).toBeGreaterThan(0);
    const width = Math.round(-x[x.length - 1]);
    expect(width).toBeGreaterThan(0);
    expectArrived(x.slice(x.findIndex((v) => v < -1)), { rest: 0, goal: -width });

    await swipe(page, 300);
    await expect(preview).toHaveAttribute('data-page', 'shortcuts');
    await expect(preview).not.toHaveAttribute('data-sliding');
    await tile(page, 'accounts_jump').click();
    await expect(preview).toHaveAttribute('data-sliding', '');
    await page.mouse.down();
    await expect(preview).not.toHaveAttribute('data-sliding');
    await page.mouse.up();
    await expect(preview).toHaveAttribute('data-page', 'accounts');
    await expect(page.getByTestId('dashboard-page')).toBeVisible();
  });

  test('the try-it Page slide and a Page tab switch keep every frame', async ({ page }) => {
    await tryIt(page);
    const preview = page.getByTestId('deck-preview');
    const slide = await frames(page);
    await swipe(page, -200);
    await expect(preview).toHaveAttribute('data-page', 'desktop');
    await expect(preview).not.toHaveAttribute('data-sliding');
    await swipe(page, 300);
    await expect(preview).toHaveAttribute('data-page', 'shortcuts');
    await expect(preview).not.toHaveAttribute('data-sliding');
    await tile(page, 'accounts_jump').click();
    await expect(preview).toHaveAttribute('data-page', 'accounts');
    await expect(preview).not.toHaveAttribute('data-sliding');
    const slid = await slide.stop();

    await page.keyboard.press('p');
    await expect(preview).toHaveAttribute('data-mode', 'edit');
    const tabs = await frames(page);
    for (const id of ['shortcuts', 'desktop', 'usage', 'accounts', 'shortcuts']) {
      await page.locator(`[data-testid="page-tab"][data-page="${id}"]`).click();
      await expect(preview).toHaveAttribute('data-page', id);
    }
    const switched = await tabs.stop();

    const measured = {
      slide_frames: slid.stamps.length,
      slide_dropped_at_60hz: droppedFrames(slid.stamps),
      slide_longest_gap_ms: longestGap(slid.stamps),
      slide_long_frames_ms: slid.long,
      tab_frames: switched.stamps.length,
      tab_dropped_at_60hz: droppedFrames(switched.stamps),
      tab_longest_gap_ms: longestGap(switched.stamps),
      tab_long_frames_ms: switched.long,
    };
    console.log(`page switch frames ${JSON.stringify(measured)}`);
    test.info().annotations.push({ type: 'frames', description: JSON.stringify(measured) });
    // Shared CI runners add 50 ms frames from contention alone; a frame over JANK_MS is the Page switch itself.
    expect(measured.slide_long_frames_ms.filter((ms) => ms > JANK_MS)).toEqual([]);
    expect(measured.tab_long_frames_ms.filter((ms) => ms > JANK_MS)).toEqual([]);
  });
});

test.describe('with reduced motion', () => {
  test.use({ reducedMotion: 'reduce' });

  test('Settings fades in without growing', async ({ page }) => {
    await home(page);
    const opened = await trace(page, [{ selector: '.dialog-settings', reading: 'scale' }, { selector: '.dialog-settings', reading: 'opacity' }]);
    await navButton(page, 'Settings').click();
    await expect(settings(page)).toHaveCSS('opacity', '1');
    await page.waitForTimeout(100);
    const [scale, opacity] = await opened.stop();
    expectHeld(scale, 1);
    expectArrived(opacity, { rest: 0, goal: 1 });
    await expect(page.locator('.dialog-settings')).toHaveCSS('transition-duration', '0.14s, 0.14s');
  });

  test('the Device panel fades in without growing, and its chevron turns at once', async ({ page }) => {
    await openLayout(page);
    const opened = await trace(page, [
      { selector: '[data-testid="device-panel"]', reading: 'scale' },
      { selector: '[data-testid="device-name"] .chev', reading: 'rotate' },
    ]);
    await page.getByTestId('device-name').click();
    await expect(page.getByTestId('device-panel')).toHaveCSS('opacity', '1');
    await page.waitForTimeout(100);
    const [scale, rotate] = await opened.stop();
    expectHeld(scale, 1);
    expectJumped(rotate, [0, 180]);
    expect(rotate.at(-1)).toBe(180);
  });

  test('a tooltip fades in without growing', async ({ page }) => {
    await home(page);
    const shown = await trace(page, [{ selector: '.tooltip', reading: 'matrix-a' }]);
    await navButton(page, 'Home').hover();
    await expect(page.locator('.tooltip')).toHaveCSS('opacity', '1');
    const [scale] = await shown.stop();
    expectHeld(scale, 1);
  });

  test('the Update icon jumps open and shut', async ({ page }) => {
    await home(page, '?update=available');
    const icon = page.locator('.update-button');
    const opened = await trace(page, [{ selector: '.update-button', reading: 'visible-width' }]);
    await icon.hover(AT_VISIBLE_ICON);
    await expect(page.locator('.update-slot')).toHaveClass(/is-expanded/);
    await page.waitForTimeout(100);
    await page.mouse.move(AWAY.x, AWAY.y);
    await expect(page.locator('.update-slot')).not.toHaveClass(/is-expanded/);
    await page.waitForTimeout(100);
    const [width] = await opened.stop();
    expectJumped(width, [44, (await widthOf(icon)) + 4]);
    await expect(page.locator('.update-label')).toHaveCSS('transition-duration', '0s');
  });

  test('a Device on Home neither lifts nor shrinks under the pointer', async ({ page }) => {
    await home(page, '?devices=1');
    const watched = await trace(page, [
      { selector: '.device-bezel', reading: 'matrix-y' },
      { selector: '.device-hit', reading: 'matrix-a' },
    ]);
    await page.locator('.device-hit').hover();
    await page.mouse.down();
    await page.waitForTimeout(200);
    await page.mouse.move(AWAY.x, 20);
    await page.mouse.up();
    const [lift, press] = await watched.stop();
    expectHeld(lift, 0);
    expectHeld(press, 1);
  });

  test("a Device's screen still fades as it connects", async ({ page }) => {
    await home(page, '?devices=1');
    await demo(page).offline('mock0001');
    await expect(page.locator('.device-scene')).toHaveCSS('opacity', '0.7');
    const lit = await trace(page, [{ selector: '.device-scene', reading: 'opacity' }]);
    await demo(page).connect('mock0001');
    await expect(page.locator('.device-scene')).toHaveCSS('opacity', '1');
    const [screen] = await lit.stop();
    expectArrived(screen, { rest: 0.7, goal: 1 });
  });

  test('a switch thumb and a dropped Tile jump to their end state', async ({ page }) => {
    await tryIt(page);
    const toggle = page.getByTestId('actual-size');
    const slid = await trace(page, [{ selector: '[data-testid="actual-size"] .switch-thumb', reading: 'translate-x' }]);
    await toggle.click();
    await expect(toggle).toHaveAttribute('data-checked');
    await page.waitForTimeout(100);
    const [thumb] = await slid.stop();
    expectJumped(thumb, [0, 14]);
    await expect(tile(page, 'f5')).toHaveCSS('transition-duration', '0s');
  });

  test('download progress steps without easing', async ({ page }) => {
    await home(page, '?update=available');
    await demo(page).updates(0.3);
    await page.locator('.update-button').click(AT_VISIBLE_ICON);
    await page.getByRole('dialog').getByRole('button', { name: 'Update' }).click();
    const fill = page.locator('.update-progress span');
    await expect(fill).toHaveCSS('transform', 'matrix(0.3, 0, 0, 1, 0, 0)');
    await expect(fill).toHaveCSS('transition-duration', '0s');
  });

  test('the try-it Page changes at once, with no slide', async ({ page }) => {
    await tryIt(page);
    const preview = page.getByTestId('deck-preview');
    await page.evaluate(() => {
      const stage = document.querySelector('[data-testid="deck-preview"]')!;
      new MutationObserver(() => {
        if (stage.hasAttribute('data-sliding')) stage.setAttribute('data-slid', '');
      }).observe(stage, { attributes: true, attributeFilter: ['data-sliding'] });
    });
    await swipe(page, -200);
    await expect(preview).toHaveAttribute('data-page', 'desktop');
    await swipe(page, 300);
    await expect(preview).toHaveAttribute('data-page', 'shortcuts');
    await tile(page, 'accounts_jump').click();
    await expect(preview).toHaveAttribute('data-page', 'accounts');
    await expect(preview).not.toHaveAttribute('data-slid');
  });
});
