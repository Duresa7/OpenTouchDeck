import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { importText, menu, openLayout, openSettings as openSettingsAt, waitEvent } from './layout-helpers.ts';

const devText = readFileSync(join(import.meta.dirname, '..', '..', '..', 'schema', 'examples', 'dev.json'), 'utf8');
const folder = process.env.TD_SHOTS ?? '';

const rail = (page: Page) => page.locator('.nav-rail');

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

/** Settings at Paired browsers, whose Reset opens a confirm. */
async function openSettings(page: Page) {
  await openHome(page);
  const settings = await openSettingsAt(page, 'browsers');
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

test('the nav rail sits beside the Settings page, not over it', async ({ page }) => {
  await openSettings(page);
  expect((await topAt(rail(page))).inside).toBe(true);
  const edge = (await rail(page).boundingBox())!;
  const categories = (await page.locator('.settings-nav').boundingBox())!;
  expect(categories.x).toBeGreaterThanOrEqual(edge.x + edge.width);
});

test('the open Update icon draws over the screen beside the rail', async ({ page }) => {
  await openHome(page, '/?update=available');
  await page.locator('.update-button').hover({ position: { x: 22, y: 22 } });
  await expect(page.locator('.update-slot')).toHaveClass(/is-expanded/);
  const label = page.locator('.update-label');
  await expect(label).toHaveCSS('opacity', '1');
  const box = (await label.boundingBox())!;
  const edge = (await rail(page).boundingBox())!;
  expect(box.x + box.width).toBeGreaterThan(edge.x + edge.width);
  const top = await page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.closest('.update-button') !== null, { x: box.x + box.width - 4, y: box.y + box.height / 2 });
  expect(top).toBe(true);
});

test.describe('the nav rail sits under every dialog', () => {
  test('the Update dialog', async ({ page }) => {
    await openUpdate(page);
    expect((await topAt(rail(page))).className).toBe('scrim');
  });

  test('the Replace confirm', async ({ page }) => {
    await openReplace(page);
    expect((await topAt(rail(page))).className).toBe('scrim');
  });
});

test('a confirm opened from Settings draws over Settings and its scrim covers it', async ({ page }) => {
  const settings = await openSettings(page);
  await settings.getByRole('button', { name: 'Reset', exact: true }).click();
  const confirm = page.getByRole('alertdialog', { name: 'Reset pairing?' });
  await expect(confirm).toBeVisible();
  expect((await topAt(confirm)).inside).toBe(true);
  expect((await topAt(page.getByTestId('settings-nav-general'))).className).toBe('scrim');
  expect((await topAt(rail(page))).className).toBe('scrim');
});

test.describe('on the Layout editor', () => {
  test.beforeEach(async ({ page }) => {
    await openEditor(page);
  });

  test('the nav rail sits beside the canvas, not over it', async ({ page }) => {
    expect((await topAt(rail(page))).inside).toBe(true);
    const edge = (await rail(page).boundingBox())!;
    const canvas = (await page.locator('.canvas').boundingBox())!;
    expect(canvas.x).toBeGreaterThanOrEqual(edge.x + edge.width);
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
