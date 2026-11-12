import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';

const folder = process.env.TD_SHOTS ?? '';

async function openIntegrations(page: Page, query = '') {
  await page.goto(`/${query}`);
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.getByRole('button', { name: 'Integrations', exact: true }).click();
  await expect(page.getByTestId('integration-list')).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}

for (const theme of ['light', 'dark'] as const) {
  test.describe(theme, () => {
    test.skip(folder === '', 'set TD_SHOTS to a folder to take screenshots');
    test.use({ colorScheme: theme });

    test('the list', async ({ page }) => {
      await openIntegrations(page);
      await expect(page.locator('[data-testid="integration"][data-id="cliproxyapi"]').getByTestId('integration-switch')).toHaveAttribute('aria-checked', 'true');
      await page.mouse.move(0, 0);
      await page.screenshot({ path: join(folder, `integrations-${theme}.png`) });
    });

    test('the list, not set up', async ({ page }) => {
      await openIntegrations(page, '?integrations=new');
      await expect(page.locator('[data-testid="integration"][data-id="cliproxyapi"]').getByTestId('integration-switch')).toHaveAttribute('aria-checked', 'false');
      await page.mouse.move(0, 0);
      await page.screenshot({ path: join(folder, `integrations-new-${theme}.png`) });
    });

    test('the list, a hub that is down', async ({ page }) => {
      await openIntegrations(page);
      await page.evaluate(() => window.touchdeckDemo?.setHub(false));
      await expect(page.locator('[data-testid="integration"][data-id="cliproxyapi"]').getByTestId('integration-status')).toHaveText("Can't connect");
      await page.mouse.move(0, 0);
      await page.screenshot({ path: join(folder, `integrations-down-${theme}.png`) });
    });

    test('CLIProxyAPI', async ({ page }) => {
      await openIntegrations(page);
      await page.locator('[data-testid="integration"][data-id="cliproxyapi"]').getByTestId('integration-settings').click();
      await expect(page.getByTestId('usage-account')).toHaveCount(2);
      await page.mouse.move(0, 0);
      await page.screenshot({ path: join(folder, `integrations-cliproxyapi-${theme}.png`) });
      await page.screenshot({ path: join(folder, `integrations-cliproxyapi-full-${theme}.png`), fullPage: true });
    });

    test('Add Tile with Preset tiles', async ({ page }) => {
      await page.goto('/');
      await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
      await page.getByRole('button', { name: 'Layout', exact: true }).click();
      await page.getByTestId('start-starter').click();
      await page.getByTestId('page-entry').filter({ hasText: 'Shortcuts' }).click();
      await page.locator('[data-testid="empty-cell"][data-row="2"][data-col="2"]').click();
      await expect(page.getByTestId('add-presets')).toBeVisible();
      await page.mouse.move(0, 0);
      await page.screenshot({ path: join(folder, `integrations-addtile-${theme}.png`) });
    });
  });
}
