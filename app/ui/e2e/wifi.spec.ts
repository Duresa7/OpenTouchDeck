import { expect, test, type Page } from '@playwright/test';

// The demo data: Stream deck by the window is linked over Wi-Fi at 192.0.2.23, Desk is
// plugged in and on the network at 192.0.2.21, and Studio isn't on the network.

const panel = (page: Page) => page.getByTestId('device-panel');
const wifiStatus = (page: Page) => page.getByTestId('device-panel-wifi-status');
const problem = (page: Page) => page.getByTestId('device-panel-wifi-problem');

async function openPanel(page: Page, name: string) {
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.getByRole('button', { name: `Edit ${name}` }).click();
  await expect(page.getByTestId('editor')).toBeVisible();
  await page.getByTestId('device-name').click();
  await expect(panel(page)).toBeVisible();
  await expect(page.getByTestId('device-panel-wifi-begin').or(page.getByTestId('device-panel-wifi-forget'))).toBeVisible();
}

async function startPairing(page: Page, address?: string) {
  await page.getByTestId('device-panel-wifi-begin').click();
  const field = page.getByTestId('device-panel-wifi-address');
  await expect(field).toBeVisible();
  if (address !== undefined) await field.fill(address);
  await page.getByTestId('device-panel-wifi-start').click();
}

test('Home and the Device settings name the Carrier', async ({ page }) => {
  await page.goto('/');
  const devices = page.getByRole('list', { name: 'Devices' });
  await expect(devices.getByRole('listitem').nth(2).locator('.device-meta .status')).toHaveText('Connected over Wi-Fi');
  await expect(devices.getByRole('listitem').nth(0).locator('.device-meta .status')).toHaveText('Connected');

  await openPanel(page, 'Stream deck by the window');
  await expect(page.getByTestId('connection')).toHaveText('Connected over Wi-Fi');
  await expect(page.getByTestId('device-panel-carrier')).toHaveText('Connected over Wi-Fi at 192.0.2.23:47820');
  await expect(wifiStatus(page)).toHaveText('Paired at 192.0.2.23:47820.');
  await expect(page.getByTestId('device-panel-usb-only')).toHaveText('Plug in Stream deck by the window with a USB cable to update or restart it.');
  await expect(page.getByTestId('device-panel-restart')).toHaveAttribute('aria-disabled', 'true');
});

test('pairs a plugged-in Device found on the network, after a wrong code', async ({ page }) => {
  await openPanel(page, 'Desk');
  await expect(page.getByTestId('device-panel-carrier')).toHaveText('Connected over USB');
  await expect(wifiStatus(page)).toHaveText('Not paired. Pair to use it without the cable.');

  await page.getByTestId('device-panel-wifi-begin').click();
  await expect(page.getByTestId('device-panel-wifi-address')).toHaveValue('192.0.2.21:47820');
  await expect(page.getByTestId('device-panel-wifi-found')).toContainText('Found Desk on the network.');
  await page.getByTestId('device-panel-wifi-start').click();

  const code = page.getByTestId('device-panel-wifi-code');
  await expect(page.getByTestId('device-panel-wifi-attempts')).toHaveText('3 tries left');
  await code.fill('000000');
  await code.press('Enter');
  await expect(problem(page)).toHaveText('Wrong code. 2 tries left.');
  await expect(page.getByTestId('device-panel-wifi-attempts')).toHaveCount(0);

  await code.fill('123456');
  await page.getByTestId('device-panel-wifi-pair').click();
  await expect(wifiStatus(page)).toHaveText('Paired at 192.0.2.21:47820.');
  await expect(problem(page)).toHaveCount(0);
  await expect(page.getByTestId('device-panel-wifi-code-form')).toHaveCount(0);
  await expect(page.getByTestId('device-panel-carrier')).toHaveText('Connected over USB');

  await page.getByTestId('device-panel-wifi-forget').click();
  await expect(wifiStatus(page)).toHaveText('Not paired. Pair to use it without the cable.');
});

test('a Device off the network takes a typed address, and nothing there says so', async ({ page }) => {
  await openPanel(page, 'Studio');
  await startPairing(page, '192.0.2.99');
  await expect(problem(page)).toHaveText('Nothing answered at that address. Check that Studio is on the same network, and the address.');
  await expect(page.getByTestId('device-panel-wifi-address')).toHaveValue('192.0.2.99');
  await page.getByTestId('device-panel-wifi-cancel').click();
  await expect(page.getByTestId('device-panel-wifi-address-form')).toHaveCount(0);
  await expect(page.getByTestId('device-panel-wifi-begin')).toBeVisible();
});

test('three wrong codes lock pairing out', async ({ page }) => {
  await openPanel(page, 'Desk');
  await startPairing(page);
  const code = page.getByTestId('device-panel-wifi-code');
  for (const left of ['2 tries left', '1 try left']) {
    await code.fill('000000');
    await code.press('Enter');
    await expect(problem(page)).toHaveText(`Wrong code. ${left}.`);
  }
  await code.fill('000000');
  await code.press('Enter');
  await expect(problem(page)).toHaveText('Too many wrong codes. Desk stopped pairing for a while. Wait, then start again.');
  await expect(page.getByTestId('device-panel-wifi-address-form')).toBeVisible();
});

test('with no keychain the error says what to do', async ({ page }) => {
  await openPanel(page, 'Desk');
  await page.evaluate(() => window.touchdeckDemo?.wifiKeychain(false));
  await startPairing(page);
  const code = page.getByTestId('device-panel-wifi-code');
  await code.fill('123456');
  await code.press('Enter');
  await expect(problem(page)).toContainText("Credential Manager isn't available");
  await expect(wifiStatus(page)).toHaveText('Not paired. Pair to use it without the cable.');
});
