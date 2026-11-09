import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { errorCard, errorDetail, importText, openLayout, openSettings, type SettingsCategory } from './layout-helpers.ts';

const folder = process.env.TD_SHOTS ?? '';

async function open(page: Page, category: SettingsCategory = 'general') {
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  const settings = await openSettings(page, category);
  if (category === 'general') await expect(settings.getByRole('switch', { name: 'Start at sign-in' })).toBeEnabled();
  if (category === 'browsers') await expect(settings.getByTestId('paired-browser')).toHaveCount(2);
  return settings;
}

for (const theme of ['light', 'dark'] as const) {
  test.describe(theme, () => {
    test.skip(!folder, 'set TD_SHOTS to a folder to take screenshots');
    test.use({ colorScheme: theme });

    test('settings', async ({ page }) => {
      await open(page);
      await page.waitForTimeout(400);
      await page.screenshot({ path: join(folder, `settings-${theme}.png`) });
    });

    test('settings, keyboard focus on Start at sign-in', async ({ page }) => {
      const settings = await open(page);
      await settings.getByRole('switch', { name: 'Start at sign-in' }).focus();
      await page.keyboard.press('Space');
      await expect(settings.getByRole('switch', { name: 'Start at sign-in' })).toBeChecked();
      await page.waitForTimeout(400);
      await page.screenshot({ path: join(folder, `settings-switch-focus-${theme}.png`) });
    });

    test('paired browsers', async ({ page }) => {
      await open(page, 'browsers');
      await page.waitForTimeout(400);
      await page.screenshot({ path: join(folder, `settings-browsers-${theme}.png`) });
    });

    test('reset pairing asks first', async ({ page }) => {
      const settings = await open(page, 'browsers');
      await settings.getByRole('button', { name: 'Reset', exact: true }).click();
      await expect(page.getByRole('alertdialog', { name: 'Reset pairing?' })).toBeVisible();
      await page.waitForTimeout(400);
      await page.screenshot({ path: join(folder, `settings-reset-${theme}.png`) });
    });

    test('troubleshooting', async ({ page }) => {
      const settings = await open(page, 'troubleshooting');
      await expect(settings.getByTestId('device-details')).toHaveCount(3);
      await page.waitForTimeout(400);
      await page.screenshot({ path: join(folder, `troubleshooting-${theme}.png`) });
    });

    test('troubleshooting, the console after a reply and a refused line', async ({ page }) => {
      const settings = await open(page, 'troubleshooting');
      const field = settings.getByRole('textbox', { name: 'Console command' });
      await field.fill('version');
      await field.press('Enter');
      await expect(settings.getByTestId('console-log').locator('[data-kind="line"]')).toHaveCount(1);
      await field.fill('reboot');
      await field.press('Enter');
      await expect(settings.getByTestId('console-forbidden')).toBeVisible();
      await settings.getByTestId('console-forbidden').scrollIntoViewIfNeeded();
      await page.screenshot({ path: join(folder, `troubleshooting-console-${theme}.png`) });
    });

    test('about', async ({ page }) => {
      await open(page, 'about');
      await page.waitForTimeout(400);
      await page.screenshot({ path: join(folder, `settings-about-${theme}.png`) });
    });

    test('an error card, then its entry under Errors', async ({ page }) => {
      await openLayout(page);
      await importText(page, '{"schema": 1', 'broken.json');
      const broken = "“broken.json” isn't a Layout file. Nothing changed.";
      await expect(errorCard(page, broken)).toBeVisible();
      await page.waitForTimeout(600);
      await page.screenshot({ path: join(folder, `error-card-${theme}.png`) });
      await expect(await errorDetail(page, broken)).toBeVisible();
      await page.mouse.move(0, 0);
      await page.waitForTimeout(400);
      await page.screenshot({ path: join(folder, `settings-errors-${theme}.png`) });
    });
  });
}
