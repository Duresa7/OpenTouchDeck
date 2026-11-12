import { readFileSync } from 'node:fs';
import { importText, tile } from './layout-helpers.ts';
import { expect, test } from '@playwright/test';
import { addTile, openStarter, send, storedAction } from './inspector-helpers.ts';

test('a radio Device offers Bluetooth for shortcuts and media keys and keeps the chosen sink', async ({ page }) => {
  await openStarter(page);
  const id = await addTile(page, 'key');
  await expect(page.getByTestId('action-sink')).toHaveValue('link');
  await page.getByTestId('action-sink').selectOption('bluetooth');
  await expect(page.getByTestId('action-shortcut')).toContainText('on the Bluetooth computer or phone');
  await send(page);
  expect(await storedAction(page, 'shortcuts', id)).toMatchObject({ sink: 'bluetooth' });
  await page.getByTestId('action-type').selectOption('consumer');
  await page.getByTestId('action-sink').selectOption('bluetooth');
  await send(page);
  expect(await storedAction(page, 'shortcuts', id)).toMatchObject({ type: 'consumer', sink: 'bluetooth' });
});

test('a Device without the advertised sink hides Bluetooth', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Edit Studio', exact: true }).click();
  await page.getByTestId('start-starter').click();
  await page.getByTestId('page-tab').filter({ hasText: 'Shortcuts' }).click();
  await addTile(page, 'key');
  await expect(page.getByTestId('action-sink')).toHaveCount(0);
});


test('an imported Bluetooth Tile on a normal Device can switch back to the Companion', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Edit Studio', exact: true }).click();
  await importText(page, readFileSync(new URL('../../../schema/examples/bluetooth.json', import.meta.url), 'utf8'), 'bluetooth.json');
  await tile(page, 'copy').click();
  await expect(page.getByTestId('action-sink')).toHaveValue('bluetooth');
  await expect(page.getByTestId('action-sink')).toHaveAttribute('aria-invalid', 'true');
  await expect(page.getByTestId('field-error')).toContainText("This Device can't send to Bluetooth. Choose This PC (Companion).");
  await page.getByTestId('action-sink').selectOption('link');
  await expect(page.getByTestId('field-error')).toHaveCount(0);
  await tile(page, 'play').click();
  await page.getByTestId('action-sink').selectOption('link');
  await send(page);
  expect(await storedAction(page, 'keys', 'copy', 'mock0002')).not.toHaveProperty('sink');
});
