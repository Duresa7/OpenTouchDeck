import { join } from 'node:path';
import { expect, test } from '@playwright/test';

const folder = process.env.TD_SHOTS;

for (const theme of ['light', 'dark'] as const) {
  test.describe(theme, () => {
    test.skip(!folder, 'set TD_SHOTS to a folder to take screenshots');
    test.use({ colorScheme: theme });

    test.beforeEach(async ({ page }) => {
      await page.goto('/');
      await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
      await page.evaluate(() => document.fonts.ready);
    });

    test('home with the demo Devices', async ({ page }) => {
      await page.screenshot({ path: join(folder!, `home-${theme}.png`) });
    });

    test('settings opened from the keyboard, focus on its first control', async ({ page }) => {
      await page.getByRole('button', { name: 'Settings' }).focus();
      await page.keyboard.press('Enter');
      const settings = page.getByTestId('settings');
      await expect(settings).toBeVisible();
      await expect(page.getByTestId('settings-title')).toBeFocused();
      await expect(settings.getByRole('switch', { name: 'Start at sign-in' })).toBeEnabled();
      await page.keyboard.press('Tab');
      await expect(settings.getByRole('switch', { name: 'Start at sign-in' })).toBeFocused();
      await expect(settings.locator(':focus-visible')).toHaveCount(1);
      await page.waitForTimeout(400);
      await page.screenshot({ path: join(folder!, `settings-focus-${theme}.png`) });
    });

    test('keyboard focus on the nav cluster', async ({ page }) => {
      await page.getByRole('button', { name: 'Home', exact: true }).focus();
      await page.keyboard.press('Tab');
      await expect(page.locator('.nav-button:focus-visible')).toHaveAttribute('aria-label', 'Layout');
      await page.screenshot({ path: join(folder!, `focus-nav-${theme}.png`) });
    });
  });
}
