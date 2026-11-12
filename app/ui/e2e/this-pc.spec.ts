import { expect, test } from '@playwright/test';

test('This PC is switched off and on from its row, and offers all its Preset tiles in Add Tile', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Integrations', exact: true }).click();
  const row = page.locator('[data-testid="integration"][data-id="this_pc"]');
  await expect(row).toContainText('This PC');
  await expect(row.getByTestId('integration-settings')).toHaveCount(0);
  const toggle = row.getByTestId('integration-switch');
  await expect(toggle).toHaveAttribute('aria-checked', 'true');
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-checked', 'false');
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-checked', 'true');

  await page.getByRole('button', { name: 'Layout', exact: true }).click();
  await page.getByTestId('start-starter').click();
  await page.getByTestId('page-entry').filter({ hasText: 'Shortcuts' }).click();
  await page.locator('[data-testid="empty-cell"][data-row="2"][data-col="2"]').click();
  await expect(page.locator('[data-testid="preset-group"][data-id="this_pc"]').getByTestId('preset')).toHaveCount(10);
});
