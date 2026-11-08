import { expect, test, type Page } from '@playwright/test';

const nav = (page: Page) => page.getByRole('navigation', { name: 'Main' });
const navButton = (page: Page, name: string) => nav(page).getByRole('button', { name, exact: true });
const settings = (page: Page) => page.getByRole('dialog', { name: 'Settings' });

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
  await expect(cards.nth(0).getByTestId('device-status')).toHaveAccessibleName('Desk: Connected over USB');
  await expect(cards.nth(0).getByTestId('device-status')).toHaveAttribute('data-tone', 'ok');
  await expect(cards.nth(2).getByTestId('device-status')).toHaveAccessibleName(/: Connected over Wi-Fi$/);
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

test('each destination is one click from the cluster', async ({ page }) => {
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
});

test('a tooltip names each button on hover', async ({ page }) => {
  for (const name of ['Home', 'Layout', 'Integrations', 'Settings']) {
    await navButton(page, name).hover();
    await expect(page.locator('.tooltip')).toHaveText(name);
  }
});

test('Settings opens over the current screen, Escape closes it, and a change is still there on reopen', async ({ page }) => {
  await navButton(page, 'Integrations').click();
  await navButton(page, 'Settings').click();
  await expect(settings(page)).toBeVisible();
  await expect(page.getByRole('heading', { level: 1, name: 'Integrations', includeHidden: true })).toBeAttached();
  await expect(page.getByRole('button', { name: 'Settings', exact: true, includeHidden: true })).toHaveClass(/is-current/);

  const dark = settings(page).getByRole('button', { name: 'Dark' });
  await dark.click();
  await expect(dark).toHaveAttribute('aria-pressed', 'true');

  await page.keyboard.press('Escape');
  await expect(settings(page)).toBeHidden();
  await expect(page.getByRole('heading', { level: 1, name: 'Integrations' })).toBeVisible();
  await expect(navButton(page, 'Settings')).toBeFocused();

  await navButton(page, 'Settings').click();
  await expect(settings(page).getByRole('button', { name: 'Dark' })).toHaveAttribute('aria-pressed', 'true');
  await expect(settings(page).getByRole('button', { name: 'Match Windows' })).toHaveAttribute('aria-pressed', 'false');
});

test('About shows the version and build the Companion reports', async ({ page }) => {
  await navButton(page, 'Settings').click();
  await expect(settings(page)).toContainText('Open TouchDeck 0.2.0');
  await expect(settings(page)).toContainText('Build demo');
});

const background = (page: Page) => page.evaluate(() => getComputedStyle(document.body).backgroundColor);
const LIGHT_MAT = 'rgb(228, 230, 233)';
const DARK_MAT = 'rgb(31, 37, 44)';

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

async function dialogScales(page: Page) {
  await page.evaluate(() => {
    const scales: string[] = [];
    const sample = () => {
      const dialog = document.querySelector('.dialog-settings');
      if (dialog) scales.push(getComputedStyle(dialog).scale);
    };
    const observer = new MutationObserver(sample);
    observer.observe(document.body, { childList: true, subtree: true, attributes: true });
    Object.assign(window, { dialogScaleProbe: { scales, observer } });
  });
  await navButton(page, 'Settings').click();
  await expect(settings(page)).toBeVisible();
  return page.evaluate(() => {
    const probe = window.dialogScaleProbe;
    if (!probe) throw new Error('dialog scale probe is missing');
    probe.observer.disconnect();
    delete window.dialogScaleProbe;
    return probe.scales;
  });
}

declare global {
  interface Window {
    dialogScaleProbe?: { scales: string[]; observer: MutationObserver };
  }
}

const grows = (scales: string[]) => scales.some((s) => s !== 'none' && s !== '1');

test('the dialog grows into place by default', async ({ page }) => {
  expect(grows(await dialogScales(page))).toBe(true);
});

test('with reduced motion the dialog appears without moving', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const scales = await dialogScales(page);
  expect(scales.length).toBeGreaterThan(0);
  expect(grows(scales)).toBe(false);
  await expect(settings(page)).toBeVisible();
});

test('Tab reaches every nav button with the visible focus ring', async ({ page }) => {
  const reached = new Map<string, { visible: boolean; outline: string }>();
  for (let i = 0; i < 12 && reached.size < 4; i++) {
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
});
