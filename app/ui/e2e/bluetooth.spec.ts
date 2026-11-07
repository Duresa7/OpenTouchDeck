import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { openLayout } from './layout-helpers.ts';

// The demo data: Desk runs the radio build with its Bluetooth keyboard on and two computers and phones bonded,
// 00:00:5e:00:53:01 connected and 00:00:5e:00:53:02 not. Studio runs the normal build.

const FIRST = '00:00:5e:00:53:01';
const SECOND = '00:00:5e:00:53:02';
const PHONE = '00:00:5e:00:53:03';

const section = (page: Page) => page.getByTestId('device-panel-bluetooth');
const keyboardStatus = (page: Page) => page.getByTestId('device-panel-bluetooth-status');
const toggle = (page: Page) => page.getByTestId('device-panel-bluetooth-switch');
const bonds = (page: Page) => page.getByTestId('device-panel-bluetooth-bond');

async function openDesk(page: Page) {
  await openLayout(page);
  await page.getByTestId('device-name').click();
  await expect(page.getByTestId('device-panel')).toBeVisible();
  await expect(bonds(page)).toHaveCount(2);
}

async function kept(page: Page) {
  return page.evaluate(() => window.touchdeckDemo?.bluetooth.state('mock0001'));
}

test('shows the keyboard with its computers and phones, and turns it off and on', async ({ page }) => {
  await openDesk(page);
  await expect(section(page)).toHaveAttribute('data-state', 'on');
  await expect(toggle(page)).toHaveAttribute('aria-checked', 'true');
  await expect(keyboardStatus(page)).toHaveText('On. Connected to a computer or phone.');
  await expect(bonds(page).nth(0)).toContainText(FIRST);
  await expect(bonds(page).nth(0)).toContainText('Connected');
  await expect(bonds(page).nth(1)).toContainText(SECOND);
  await expect(bonds(page).nth(1)).toContainText('Not connected');

  await toggle(page).click();
  await expect(section(page)).toHaveAttribute('data-state', 'off');
  await expect(keyboardStatus(page)).toHaveText('Off. Turn it on to type on a computer or phone with Desk.');
  await expect(bonds(page)).toHaveCount(0);
  await expect(page.getByTestId('device-panel-bluetooth-add')).toHaveCount(0);
  expect(await kept(page)).toMatchObject({ enabled: false, bonds: [{ address: FIRST }, { address: SECOND }] });

  await toggle(page).click();
  await expect(keyboardStatus(page)).toHaveText('Starting…');
  await expect(section(page)).toHaveAttribute('data-state', 'on');
  await expect(bonds(page)).toHaveCount(2);
});

test('adds a phone with the code on the Device, and the new Keyboard host closes the window', async ({ page }) => {
  await openDesk(page);
  await page.getByTestId('device-panel-bluetooth-add').click();
  const window_ = page.getByTestId('device-panel-bluetooth-window');
  await expect(window_).toBeVisible();
  await expect(window_).toContainText('Open Bluetooth on the computer or phone and choose Desk. Desk shows a 6-digit code; type it on the computer or phone.');
  await expect(page.getByTestId('device-panel-bluetooth-countdown')).toHaveText(/^(2:00|1:5\d)$/u);
  await expect(keyboardStatus(page)).toHaveText('Waiting for a computer or phone…');

  await page.evaluate(() => window.touchdeckDemo?.bluetooth.passkey('mock0001', true));
  await expect(window_).toHaveAttribute('data-passkey', 'true');
  await expect(keyboardStatus(page)).toHaveText('Desk shows a 6-digit code. Type it on the computer or phone.');
  await expect(section(page)).not.toContainText(/\b\d{6}\b/u);

  await page.evaluate((address) => window.touchdeckDemo?.bluetooth.pair('mock0001', address), PHONE);
  await expect(window_).toHaveCount(0);
  await expect(bonds(page)).toHaveCount(3);
  await expect(bonds(page).nth(2)).toHaveAttribute('data-address', PHONE);
  await expect(bonds(page).nth(2)).toHaveAttribute('data-connected', 'true');
});

test('stops adding', async ({ page }) => {
  await openDesk(page);
  await page.getByTestId('device-panel-bluetooth-add').click();
  await expect(page.getByTestId('device-panel-bluetooth-window')).toBeVisible();
  await page.getByTestId('device-panel-bluetooth-stop').click();
  await expect(page.getByTestId('device-panel-bluetooth-window')).toHaveCount(0);
  await expect(page.getByTestId('device-panel-bluetooth-add')).toBeVisible();
  await expect(keyboardStatus(page)).toHaveText('On. Connected to a computer or phone.');
});

test('forgets one computer or phone, then all of them after a confirmation', async ({ page }) => {
  await openDesk(page);
  await page.getByRole('button', { name: `Forget ${FIRST}` }).click();
  await expect(bonds(page)).toHaveCount(1);
  await expect(bonds(page).first()).toHaveAttribute('data-address', SECOND);
  await expect(keyboardStatus(page)).toHaveText('On. Waiting for a computer or phone to connect.');

  await page.getByTestId('device-panel-bluetooth-forget-all').click();
  const confirm = page.getByTestId('device-panel-bluetooth-confirm');
  await expect(confirm).toContainText('Forget every computer and phone?');
  await page.getByTestId('device-panel-bluetooth-confirm-cancel').click();
  await expect(confirm).toHaveCount(0);
  await expect(bonds(page)).toHaveCount(1);

  await page.getByTestId('device-panel-bluetooth-forget-all').click();
  await page.getByTestId('device-panel-bluetooth-confirm-forget').click();
  await expect(bonds(page)).toHaveCount(0);
  await expect(confirm).toHaveCount(0);
  await expect(page.getByTestId('device-panel-bluetooth-forget-all')).toHaveCount(0);
  await expect(keyboardStatus(page)).toHaveText('On. Add a computer or phone to type with Desk.');
  expect(await kept(page)).toMatchObject({ enabled: true, bonds: [] });
});

test('a radio that will not start says so', async ({ page }) => {
  await openDesk(page);
  await toggle(page).click();
  await expect(section(page)).toHaveAttribute('data-state', 'off');
  await page.evaluate(() => window.touchdeckDemo?.bluetooth.failStart(true));
  await toggle(page).click();
  await expect(page.getByTestId('device-panel-bluetooth-error')).toHaveText("Bluetooth didn't start on Desk. Restart Desk to try again.");
  await expect(section(page)).toHaveAttribute('data-state', 'failed');
  // Turning it off after a failed start reads off, not failed.
  await page.evaluate(() => window.touchdeckDemo?.bluetooth.failStart(false));
  await toggle(page).click();
  await expect(section(page)).toHaveAttribute('data-state', 'off');
  await expect(keyboardStatus(page)).toHaveText('Off. Turn it on to type on a computer or phone with Desk.');
  await expect(page.getByTestId('device-panel-bluetooth-error')).toHaveCount(0);
});

test('over Wi-Fi the section is read-only and asks for a USB cable', async ({ page }) => {
  await openDesk(page);
  await page.evaluate(() =>
    window.touchdeckDemo?.device('mock0001', { carrier: 'wifi', link: { state: 'connected', port: '192.0.2.21:47820', since_ms: 0, protocol: 2 } }),
  );
  await expect(section(page)).toHaveAttribute('data-usb', 'false');
  await expect(keyboardStatus(page)).toHaveText('Plug in Desk with a USB cable to change its Bluetooth keyboard.');
  await expect(toggle(page)).toHaveAttribute('data-disabled', '');
  for (const id of ['device-panel-bluetooth-add', 'device-panel-bluetooth-forget-all']) {
    await expect(page.getByTestId(id)).toHaveAttribute('aria-disabled', 'true');
  }
  // aria-disabled buttons stay focusable, so a click still lands and must do nothing.
  await page.getByTestId('device-panel-bluetooth-forget-all').click({ force: true });
  await expect(page.getByTestId('device-panel-bluetooth-confirm')).toHaveCount(0);
  await page.getByRole('button', { name: `Forget ${FIRST}` }).click({ force: true });
  await expect(bonds(page)).toHaveCount(2);
  expect(await kept(page)).toMatchObject({ enabled: true, bonds: [{ address: FIRST }, { address: SECOND }] });
});

test('a Device without the Bluetooth keyboard shows no section', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.getByRole('button', { name: 'Edit Studio' }).click();
  await expect(page.getByTestId('editor')).toHaveAttribute('data-device-id', 'mock0002');
  await page.getByTestId('device-name').click();
  await expect(page.getByTestId('device-panel-update')).toBeVisible();
  await expect(section(page)).toHaveCount(0);
});

const folder = process.env.TD_SHOTS;

for (const theme of ['light', 'dark'] as const) {
  test.describe(`shots ${theme}`, () => {
    test.skip(!folder, 'set TD_SHOTS to a folder to take screenshots');
    test.use({ colorScheme: theme });

    async function shoot(page: Page, name: string) {
      await page.mouse.move(0, 0);
      await page.waitForTimeout(300);
      await section(page).scrollIntoViewIfNeeded();
      await page.getByTestId('device-panel').screenshot({ path: join(folder!, `bluetooth-${name}-${theme}.png`) });
    }

    test('the keyboard with two computers and phones', async ({ page }) => {
      await openDesk(page);
      await shoot(page, 'bonds');
    });

    test('the pairing window with the code up', async ({ page }) => {
      await openDesk(page);
      await page.getByTestId('device-panel-bluetooth-add').click();
      await page.evaluate(() => window.touchdeckDemo?.bluetooth.passkey('mock0001', true));
      await expect(page.getByTestId('device-panel-bluetooth-window')).toHaveAttribute('data-passkey', 'true');
      await shoot(page, 'pairing');
    });

    test('forget all, confirming', async ({ page }) => {
      await openDesk(page);
      await page.getByTestId('device-panel-bluetooth-forget-all').click();
      await expect(page.getByTestId('device-panel-bluetooth-confirm')).toBeVisible();
      await shoot(page, 'forget-all');
    });

    test('read-only over Wi-Fi', async ({ page }) => {
      await openDesk(page);
      await page.evaluate(() =>
        window.touchdeckDemo?.device('mock0001', { carrier: 'wifi', link: { state: 'connected', port: '192.0.2.21:47820', since_ms: 0, protocol: 2 } }),
      );
      await expect(section(page)).toHaveAttribute('data-usb', 'false');
      await shoot(page, 'wifi');
    });
  });
}
