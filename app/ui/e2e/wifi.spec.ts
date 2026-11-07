import { expect, test, type Page } from '@playwright/test';
import { openLayout } from './layout-helpers.ts';

// Desk (mock0001) runs the radio build in the demo; Studio (mock0002) runs the normal build. The fake Companion's
// networks and every password here are made up.
const NETWORK = 'td-demo-network';
const PASSWORD = 'demo-pass-1234';

const wifiStatus = (page: Page) => page.getByTestId('device-panel-wifi-status');

async function openPanel(page: Page) {
  await openLayout(page);
  await page.getByTestId('device-name').click();
  await expect(page.getByTestId('device-panel')).toBeVisible();
  await expect(wifiStatus(page)).toHaveAttribute('data-state', 'off');
}

async function join(page: Page, ssid: string, password: string) {
  await page.getByTestId('device-panel-wifi-ssid').fill(ssid);
  await page.getByTestId('device-panel-wifi-password').fill(password);
  await page.getByTestId('device-panel-wifi-join').click();
}

async function stored(page: Page): Promise<{ ssid: string; password: string } | null> {
  return page.evaluate(async () => (await fetch('/api/v1/test/devices/mock0001/wifi')).json());
}

test('joins a network, and the password field empties and never comes back', async ({ page }) => {
  await openPanel(page);
  await expect(page.getByTestId('device-panel-wifi-password')).toHaveAttribute('type', 'password');
  await join(page, NETWORK, PASSWORD);
  await expect(wifiStatus(page)).toHaveAttribute('data-state', 'connecting');
  await expect(wifiStatus(page)).toHaveText(`Joining ${NETWORK}…`);
  await expect(wifiStatus(page)).toHaveAttribute('data-state', 'connected');
  await expect(wifiStatus(page)).toHaveText(`Connected to ${NETWORK} at 192.0.2.10`);
  await expect(page.getByTestId('device-panel-wifi-password')).toHaveValue('');
  await expect(page.getByTestId('device-panel-wifi-ssid')).toHaveValue(NETWORK);
  expect(await stored(page)).toEqual({ ssid: NETWORK, password: PASSWORD });
  expect(await page.content()).not.toContain(PASSWORD);

  await page.keyboard.press('Escape');
  await page.getByTestId('device-name').click();
  await expect(wifiStatus(page)).toHaveAttribute('data-state', 'connected');
  await expect(page.getByTestId('device-panel-wifi-password')).toHaveValue('');
});

test('says why a join failed', async ({ page }) => {
  await openPanel(page);
  await join(page, 'td-test-nonexistent', PASSWORD);
  await expect(wifiStatus(page)).toHaveAttribute('data-reason', 'not-found');
  await expect(wifiStatus(page)).toHaveText("Can't find td-test-nonexistent. Check the name, and that Desk is in range. Desk keeps trying.");
  await join(page, NETWORK, 'wrong-pass-5678');
  await expect(wifiStatus(page)).toHaveAttribute('data-reason', 'wrong-password');
  await expect(wifiStatus(page)).toHaveText(`${NETWORK} refused the password. Desk keeps trying.`);
});

test('refuses a short password before sending anything', async ({ page }) => {
  await openPanel(page);
  await join(page, NETWORK, 'short');
  await expect(page.getByTestId('device-panel-wifi-password-hint')).toHaveText('A Wi-Fi password has at least 8 characters.');
  await expect(page.getByTestId('device-panel-wifi-password')).toHaveAttribute('aria-invalid', 'true');
  await expect(wifiStatus(page)).toHaveAttribute('data-state', 'off');
  expect(await stored(page)).toBeNull();
});

test('forgets the network', async ({ page }) => {
  await openPanel(page);
  await expect(page.getByTestId('device-panel-wifi-forget')).toHaveCount(0);
  await join(page, 'td-demo-open', '');
  await expect(wifiStatus(page)).toHaveAttribute('data-state', 'connected');
  await page.getByTestId('device-panel-wifi-forget').click();
  await expect(wifiStatus(page)).toHaveAttribute('data-state', 'off');
  await expect(page.getByTestId('device-panel-wifi-ssid')).toHaveValue('');
  expect(await stored(page)).toBeNull();
});

test('a Device on the normal build shows no Wi-Fi section', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.getByRole('button', { name: 'Edit Studio' }).click();
  await expect(page.getByTestId('editor')).toHaveAttribute('data-device-id', 'mock0002');
  await page.getByTestId('device-name').click();
  await expect(page.getByTestId('device-panel-name')).toBeEnabled();
  await expect(page.getByTestId('device-panel-wifi')).toHaveCount(0);
});
