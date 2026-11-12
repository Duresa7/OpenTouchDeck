import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { importText, openSettings, settingsPage, waitEvent } from './layout-helpers.ts';

const nav = (page: Page) => page.getByRole('navigation', { name: 'Main' });
const navButton = (page: Page, name: string) => nav(page).getByRole('button', { name, exact: true });
const settings = settingsPage;

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
});

test('the window is titled Open TouchDeck and opens on Home with the Devices and their status', async ({ page }) => {
  await expect(page).toHaveTitle('Open TouchDeck');
  await expect(navButton(page, 'Home')).toHaveAttribute('aria-current', 'page');
  const cards = page.getByRole('listitem');
  await expect(cards).toHaveCount(3);
  await expect(cards.nth(0)).toContainText('Desk');
  await expect(cards.nth(0).getByTestId('device-status')).toHaveText('Desk: Connected over USB');
  await expect(cards.nth(0).getByTestId('device-status')).toHaveAttribute('data-tone', 'ok');
  await expect(cards.nth(2).getByTestId('device-status')).toHaveText(/: Connected over Wi-Fi$/);
});

test('the shell runs without console errors or warnings', async ({ page }) => {
  const problems: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error' || message.type() === 'warning') problems.push(message.text());
  });
  page.on('pageerror', (error) => problems.push(error.message));
  await page.reload();
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  for (const name of ['Layout', 'Integrations', 'Home', 'Settings']) await navButton(page, name).click();
  await page.keyboard.press('Escape');
  expect(problems).toEqual([]);
});

test('each destination is one click from the rail', async ({ page }) => {
  await navButton(page, 'Layout').click();
  await expect(page.getByRole('heading', { level: 1, name: 'Layout' })).toBeVisible();
  await expect(navButton(page, 'Layout')).toHaveAttribute('aria-current', 'page');

  await navButton(page, 'Integrations').click();
  await expect(page.getByRole('heading', { level: 1, name: 'Integrations' })).toBeVisible();
  await expect(navButton(page, 'Layout')).not.toHaveAttribute('aria-current');

  await navButton(page, 'Home').click();
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();

  await navButton(page, 'Settings').click();
  await expect(settings(page)).toBeVisible();
  await expect(navButton(page, 'Settings')).toHaveAttribute('aria-current', 'page');
  await expect(navButton(page, 'Home')).not.toHaveAttribute('aria-current');
});

test('a tooltip to the right of each button names it on hover', async ({ page }) => {
  for (const name of ['Home', 'Layout', 'Integrations', 'Settings']) {
    await navButton(page, name).hover();
    await expect(page.locator('.tooltip')).toHaveText(name);
    const button = (await navButton(page, name).boundingBox())!;
    await expect
      .poll(async () => {
        const tip = (await page.locator('.tooltip').boundingBox())!;
        return { gap: Math.round(tip.x - (button.x + button.width)), middle: Math.abs(tip.y + tip.height / 2 - (button.y + button.height / 2)) < 1 };
      })
      .toEqual({ gap: 8, middle: true });
  }
});

const rail = (page: Page) => page.locator('.nav-rail');

test('the rail runs down the left edge for the full height, and no screen moves it or lays out under it', async ({ page }) => {
  for (const size of [{ width: 800, height: 600 }, { width: 1280, height: 800 }]) {
    await page.setViewportSize(size);
    const look = await rail(page).evaluate((el) => {
      const style = getComputedStyle(el);
      return { background: style.backgroundColor, edge: style.boxShadow };
    });
    const tokens = await page.evaluate(() => {
      const probe = document.createElement('span');
      probe.style.backgroundColor = 'var(--surface)';
      probe.style.color = 'var(--line)';
      document.body.append(probe);
      const style = getComputedStyle(probe);
      const colors = { surface: style.backgroundColor, line: style.color };
      probe.remove();
      return colors;
    });
    // The edge is drawn inside the rail's 64px, so nothing to its right sits under it.
    expect(look).toEqual({ background: tokens.surface, edge: `${tokens.line} -1px 0px 0px 0px inset` });
    const where = { x: 0, y: 0, width: 64, height: size.height };
    for (const name of ['Home', 'Layout', 'Integrations', 'Settings']) {
      await navButton(page, name).click();
      await expect(navButton(page, name)).toHaveAttribute('aria-current', 'page');
      await expect.poll(() => rail(page).boundingBox(), { message: `the rail on ${name}` }).toEqual(where);
      const screen = await page.locator('.shell-screen > *').first().boundingBox();
      expect(screen!.x, `${name} starts right of the rail`).toBeGreaterThanOrEqual(64);
      expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth), `${name} scrolls sideways`).toBe(0);
    }
  }
});

test('the rail keeps its place in the Layout editor as the Details panel hides and comes back', async ({ page }) => {
  await navButton(page, 'Layout').click();
  await expect(page.getByTestId('editor')).toBeVisible();
  await importText(page, readFileSync(join(import.meta.dirname, '..', '..', '..', 'schema', 'examples', 'dev.json'), 'utf8'), 'dev.json');
  await waitEvent(page, 'opened');
  const where = await rail(page).boundingBox();
  const home = await navButton(page, 'Home').boundingBox();
  const toggle = page.getByTestId('details-toggle');
  for (const expanded of ['false', 'true']) {
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-expanded', expanded);
    // Sample through the panel's slide, not only once it settles.
    for (let i = 0; i < 6; i++) {
      expect(await rail(page).boundingBox()).toEqual(where);
      expect(await navButton(page, 'Home').boundingBox()).toEqual(home);
      await page.waitForTimeout(50);
    }
  }
});

test('Settings lays out its category column to the right of the rail', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 600 });
  await navButton(page, 'Settings').click();
  const categories = (await page.locator('.settings-nav').boundingBox())!;
  expect(categories.x).toBe(64);
  expect(categories.width).toBe(232);
});

test('Settings is a page of its own: it takes the place of the current screen, Escape leaves it open, and a change is still there on return', async ({ page }) => {
  await navButton(page, 'Integrations').click();
  await navButton(page, 'Settings').click();
  await expect(settings(page)).toBeVisible();
  await expect(page.getByRole('heading', { level: 1, name: 'Integrations', includeHidden: true })).toHaveCount(0);
  await expect(navButton(page, 'Settings')).toHaveClass(/is-current/);
  await expect(navButton(page, 'Settings')).toHaveAttribute('aria-current', 'page');
  await expect(navButton(page, 'Integrations')).not.toHaveAttribute('aria-current');

  const dark = settings(page).getByRole('button', { name: 'Dark' });
  await dark.click();
  await expect(dark).toHaveAttribute('aria-pressed', 'true');

  await page.keyboard.press('Escape');
  await expect(settings(page)).toBeVisible();

  await navButton(page, 'Integrations').click();
  await expect(page.getByRole('heading', { level: 1, name: 'Integrations' })).toBeVisible();
  await expect(settings(page)).toHaveCount(0);
  await expect(navButton(page, 'Settings')).not.toHaveAttribute('aria-current');

  await navButton(page, 'Settings').click();
  await expect(settings(page).getByRole('button', { name: 'Dark' })).toHaveAttribute('aria-pressed', 'true');
  await expect(settings(page).getByRole('button', { name: 'Match Windows' })).toHaveAttribute('aria-pressed', 'false');
});

test('About shows the version and build the Companion reports', async ({ page }) => {
  await openSettings(page, 'about');
  await expect(settings(page)).toContainText('Open TouchDeck 0.2.0');
  await expect(settings(page)).toContainText('Build demo');
});

const background = (page: Page) => page.evaluate(() => getComputedStyle(document.body).backgroundColor);
const LIGHT_MAT = 'rgb(228, 230, 233)';
const DARK_MAT = 'rgb(0, 0, 0)';

test('light and dark follow the Windows setting', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'light' });
  await expect.poll(() => background(page)).toBe(LIGHT_MAT);
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect.poll(() => background(page)).toBe(DARK_MAT);
});

test('forcing a theme with Appearance sets data-theme over the Windows setting', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'light' });
  const html = page.locator('html');
  await expect(html).not.toHaveAttribute('data-theme');

  await navButton(page, 'Settings').click();
  await settings(page).getByRole('button', { name: 'Dark' }).click();
  await expect(html).toHaveAttribute('data-theme', 'dark');
  await expect.poll(() => background(page)).toBe(DARK_MAT);

  await page.emulateMedia({ colorScheme: 'dark' });
  await settings(page).getByRole('button', { name: 'Light' }).click();
  await expect(html).toHaveAttribute('data-theme', 'light');
  await expect.poll(() => background(page)).toBe(LIGHT_MAT);

  await settings(page).getByRole('button', { name: 'Match Windows' }).click();
  await expect(html).not.toHaveAttribute('data-theme');
  await expect.poll(() => background(page)).toBe(DARK_MAT);
});

test('Tab reaches the rail before the screen, and every nav button with the visible focus ring', async ({ page }) => {
  const reached = new Map<string, { visible: boolean; outline: string }>();
  for (let i = 0; i < 4; i++) {
    await page.keyboard.press('Tab');
    const focused = await page.evaluate(() => {
      const el = document.activeElement;
      if (!el?.classList.contains('nav-button')) return null;
      const style = getComputedStyle(el);
      return {
        name: el.getAttribute('aria-label') ?? '',
        visible: el.matches(':focus-visible'),
        outline: `${style.outlineStyle} ${style.outlineWidth}`,
      };
    });
    if (focused) reached.set(focused.name, focused);
  }
  expect([...reached.keys()]).toEqual(['Home', 'Layout', 'Integrations', 'Settings']);
  for (const focus of reached.values()) {
    expect(focus.visible).toBe(true);
    expect(focus.outline).toBe('solid 2px');
  }
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'Edit Desk' })).toBeFocused();
});
