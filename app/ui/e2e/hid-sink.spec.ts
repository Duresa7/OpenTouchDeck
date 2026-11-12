import { expect, test } from '@playwright/test';
import { addTile, openStarter, send, storedAction } from './inspector-helpers.ts';

test('a radio Device offers Bluetooth for shortcuts and media keys and keeps the chosen sink', async ({ page }) => {
  await openStarter(page);
  const id = await addTile(page, 'key');
  await expect(page.getByTestId('action-sink')).toHaveValue('link');
  await page.getByTestId('action-sink').selectOption('hid');
  await expect(page.getByTestId('action-shortcut')).toContainText('on the Bluetooth computer or phone');
  await send(page);
  expect(await storedAction(page, 'shortcuts', id)).toMatchObject({ sink: 'hid' });
  await page.getByTestId('action-type').selectOption('consumer');
  await page.getByTestId('action-sink').selectOption('hid');
  await send(page);
  expect(await storedAction(page, 'shortcuts', id)).toMatchObject({ type: 'consumer', sink: 'hid' });
});

test('a Device without the advertised sink hides Bluetooth', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Edit Studio', exact: true }).click();
  await page.getByTestId('start-starter').click();
  await page.getByTestId('page-tab').filter({ hasText: 'Shortcuts' }).click();
  await addTile(page, 'key');
  await expect(page.getByTestId('action-sink')).toHaveCount(0);
});
