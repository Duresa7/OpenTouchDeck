import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { importText, menu, openLayout, waitEvent } from './layout-helpers.ts';

const devText = readFileSync(join(import.meta.dirname, '..', '..', '..', 'schema', 'examples', 'dev.json'), 'utf8');
const folder = process.env.TD_SHOTS ?? '';

const dock = (page: Page) => page.locator('.nav-dock');

function topAt(target: Locator) {
  return target.evaluate((el) => {
    const box = el.getBoundingClientRect();
    const stack = document.elementsFromPoint(box.x + box.width / 2, box.y + box.height / 2);
    const top = stack.at(0);
    return {
      inside: top !== undefined && el.contains(top),
      className: top?.getAttribute('class') ?? null,
      below: stack.slice(1).map((under) => under.getAttribute('class') ?? ''),
    };
  });
}

async function openHome(page: Page, path = '/') {
  await page.goto(path);
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}

async function openSettings(page: Page) {
  await openHome(page);
  await page.getByRole('button', { name: 'Settings' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await expect(settings.getByTestId('paired-browser')).toHaveCount(2);
  return settings;
}

async function openUpdate(page: Page) {
  await openHome(page, '/?update=available');
  await page.locator('.update-button').click({ position: { x: 22, y: 22 } });
  await expect(page.getByRole('dialog')).toHaveAccessibleName('Open TouchDeck 1.1.0');
}

async function openEditor(page: Page) {
  await openLayout(page);
  await importText(page, devText, 'dev.json');
  await waitEvent(page, 'opened');
}

async function openReplace(page: Page) {
  await openEditor(page);
  await menu(page, 'pull');
  const confirm = page.getByRole('alertdialog');
  await expect(confirm).toContainText('Replace the landscape Layout here with it?');
  return confirm;
}

test.describe('the nav dock sits under every dialog', () => {
  test('Settings', async ({ page }) => {
    await openSettings(page);
    expect((await topAt(dock(page))).className).toBe('scrim');
  });

  test('the Update dialog', async ({ page }) => {
    await openUpdate(page);
    expect((await topAt(dock(page))).className).toBe('scrim');
  });

  test('the Replace confirm', async ({ page }) => {
    await openReplace(page);
    expect((await topAt(dock(page))).className).toBe('scrim');
  });
});

test('a confirm opened from Settings draws over Settings and its scrim covers it', async ({ page }) => {
  const settings = await openSettings(page);
  const close = page.locator('.dialog-settings [aria-label="Close"]');
  await settings.getByRole('button', { name: 'Reset', exact: true }).click();
  const confirm = page.getByRole('alertdialog', { name: 'Reset pairing?' });
  await expect(confirm).toBeVisible();
  expect((await topAt(confirm)).inside).toBe(true);
  // Clicking Reset may scroll the Settings dialog, which is taller than the window, so its head
  // goes out of view. Bring it back to see what covers its Close button.
  await close.evaluate((el) => el.closest('.dialog-settings')?.scrollTo(0, 0));
  expect((await topAt(close)).className).toBe('scrim');
  expect((await topAt(dock(page))).className).toBe('scrim');
});

test.describe('on the Layout editor', () => {
  test.beforeEach(async ({ page }) => {
    await openEditor(page);
  });

  test('the nav dock draws over the canvas', async ({ page }) => {
    const at = await topAt(dock(page));
    expect(at.inside).toBe(true);
    expect(at.below).toContain('canvas');
  });

  test('the Layout menu draws over the editor', async ({ page }) => {
    await page.getByTestId('layout-menu').click();
    const popup = page.getByRole('menu', { name: 'Layout options' });
    await expect(popup).toBeVisible();
    expect((await topAt(popup)).inside).toBe(true);
  });

  test('a tooltip draws over the editor', async ({ page }) => {
    await page.getByTestId('layout-menu').hover();
    const tip = page.locator('.tooltip', { hasText: 'More' });
    await expect(tip).toBeVisible();
    expect((await topAt(tip)).inside).toBe(true);
  });
});

for (const theme of ['light', 'dark'] as const) {
  test.describe(`${theme} screenshots`, () => {
    test.skip(!folder, 'set TD_SHOTS to a folder to take screenshots');
    test.use({ colorScheme: theme });

    const shot = (page: Page, name: string) => page.screenshot({ path: join(folder, `${name}-${theme}.png`) });

    test('Home', async ({ page }) => {
      await openHome(page);
      await shot(page, 'home');
    });

    test('the Layout editor', async ({ page }) => {
      await openEditor(page);
      await shot(page, 'layout');
    });

    test('Settings open', async ({ page }) => {
      await openSettings(page);
      await page.waitForTimeout(400);
      await shot(page, 'settings');
    });

    test('the Replace confirm', async ({ page }) => {
      await openReplace(page);
      await page.waitForTimeout(400);
      await shot(page, 'replace');
    });
  });
}
