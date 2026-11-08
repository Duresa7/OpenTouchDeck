import { expect, test } from '@playwright/test';

test('Custom accepts JSON source settings and offers number Presets', async ({ page }, testInfo) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Integrations', exact: true }).click();
  const custom = page.locator('[data-testid="integration"][data-id="custom"]');
  await custom.getByTestId('integration-settings').click();
  const sources = page.getByRole('textbox', { name: 'Custom sources (JSON)' });
  await expect(sources).toHaveValue('[]');
  await sources.fill('[broken');
  await page.getByTestId('config-save').click();
  await expect(page.getByTestId('field-error')).toHaveText('Enter valid JSON.');
  await sources.fill(JSON.stringify([{ id: 'temperature', name: 'Room', kind: 'number', interval_s: 5, input: { type: 'http', url: 'https://example.invalid/metrics', value_path: '$.value' } }]));
  await page.getByTestId('config-save').click();
  await expect(page.locator('[data-testid="outcome"][data-at="settings"]')).toHaveText('Saved');
  await page.getByTestId('detail-switch').click();
  await expect(page.getByTestId('detail-switch')).toHaveAttribute('aria-checked', 'true');
  await page.screenshot({ path: testInfo.outputPath('custom.png'), fullPage: true });
  await page.reload();
  await page.getByRole('button', { name: 'Integrations', exact: true }).click();
  await custom.getByTestId('integration-settings').click();
  await expect(sources).toHaveValue(/temperature/);
  await sources.fill('[]');
  await page.getByTestId('config-save').click();
  await expect(page.locator('[data-testid="outcome"][data-at="settings"]')).toHaveText('Saved');
});
