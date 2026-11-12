import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';

const folder = process.env.TD_SHOTS;

async function openLayout(page: Page) {
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.getByRole('button', { name: 'Layout', exact: true }).click();
  await expect(page.getByTestId('editor')).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}

for (const theme of ['light', 'dark'] as const) {
  test.describe(theme, () => {
    test.skip(!folder, 'set TD_SHOTS to a folder to take screenshots');
    test.use({ colorScheme: theme });

    test('empty state', async ({ page }) => {
      await openLayout(page);
      await expect(page.getByTestId('start-layout')).toBeVisible();
      await page.screenshot({ path: join(folder!, `layout-empty-${theme}.png`) });
    });

    test('an action Tile selected', async ({ page }) => {
      await openLayout(page);
      await page.getByTestId('start-starter').click();
      await page.locator('[data-testid="tile"][data-id="playpause"]').click();
      await expect(page.getByTestId('tile-title')).toHaveText('Play/Pause');
      await page.mouse.move(0, 0);
      await page.screenshot({ path: join(folder!, `layout-action-${theme}.png`) });
    });

    test('Add Tile open', async ({ page }) => {
      await openLayout(page);
      await page.getByTestId('start-starter').click();
      await page.getByTestId('page-entry').filter({ hasText: 'Shortcuts' }).click();
      await page.locator('[data-testid="empty-cell"][data-row="2"][data-col="2"]').click();
      await expect(page.getByTestId('add-where')).toHaveText('Row 3, column 3 on the Shortcuts Page');
      await page.mouse.move(0, 0);
      await page.screenshot({ path: join(folder!, `layout-add-${theme}.png`) });
    });
  });
}
