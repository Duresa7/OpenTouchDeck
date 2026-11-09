import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { openDeviceSettings, openLayout, settingsPage } from './layout-helpers.ts';

// In the demo, Desk (mock0001) runs the opt-in radio build and Studio (mock0002) the normal build.
const build = (page: Page) => page.getByTestId('device-panel-build');
const current = (page: Page) => page.getByTestId('device-panel-build-current');
const toggle = (page: Page) => page.getByTestId('device-panel-build-switch');
const confirm = (page: Page) => page.getByTestId('device-panel-build-confirm');
const note = (page: Page) => page.getByTestId('device-panel-build-note');
const status = (page: Page) => page.getByTestId('device-panel-firmware-status');

async function openPanel(page: Page, device: 'Desk' | 'Studio', { starter = false } = {}) {
  await openLayout(page);
  if (device === 'Studio') {
    await page.getByRole('button', { name: 'Home', exact: true }).click();
    await page.getByRole('button', { name: 'Edit Studio' }).click();
    await expect(page.getByTestId('editor')).toHaveAttribute('data-device-id', 'mock0002');
  }
  if (starter) await page.getByTestId('start-starter').click();
  await openDeviceSettings(page);
  await expect(toggle(page)).toBeVisible();
}

test('a Device on the radio build switches back after a warning that its Wi-Fi and Bluetooth setup stays unused', async ({ page }) => {
  await openPanel(page, 'Desk');
  await expect(build(page)).toHaveAttribute('data-channel', 'radio');
  await expect(current(page)).toHaveText('Radio build');
  await expect(toggle(page)).toHaveText('Switch to the normal build');
  await toggle(page).click();
  await expect(confirm(page)).toContainText('Switch Desk back to the normal build?');
  await expect(note(page)).toHaveText('The Wi-Fi and Bluetooth setup stays stored on Desk but unused. Desk restarts once.');
  await expect(note(page)).toHaveClass(/is-warning/);
  await page.getByTestId('device-panel-build-confirm-switch').click();
  await expect(confirm(page)).toHaveCount(0);
  await expect(status(page)).toHaveText('Now on the normal build, 1.5.0', { timeout: 10_000 });
  await expect(build(page)).toHaveAttribute('data-channel', 'normal');
  await expect(current(page)).toHaveText('Normal build');
  await expect(toggle(page)).toHaveText('Install the radio build');
  await expect(settingsPage(page).locator('.page-lede')).toHaveText('Connected over USB');
});

test('a Device on the normal build offers the radio build and says Wi-Fi and Bluetooth stay off until set up', async ({ page }) => {
  await openPanel(page, 'Studio');
  await expect(build(page)).toHaveAttribute('data-channel', 'normal');
  await expect(current(page)).toHaveText('Normal build');
  await expect(page.getByTestId('device-panel-build-hint')).toHaveText(
    'The radio build adds Wi-Fi and Bluetooth, both off until you set them up. You can switch back any time.',
  );
  await toggle(page).click();
  await expect(confirm(page)).toContainText('Install the radio build on Studio?');
  await expect(note(page)).toHaveText('It adds Wi-Fi and Bluetooth, both off until you set them up. Studio restarts once.');
  await expect(note(page)).not.toHaveClass(/is-warning/);
  await page.getByTestId('device-panel-build-confirm-switch').click();
  await expect(status(page)).toHaveText('Now on the radio build, 1.5.0', { timeout: 10_000 });
  await expect(build(page)).toHaveAttribute('data-channel', 'radio');
  await expect(toggle(page)).toHaveText('Switch to the normal build');
});

test('Cancel keeps the build and starts nothing', async ({ page }) => {
  await openPanel(page, 'Desk');
  await toggle(page).click();
  await expect(confirm(page)).toBeVisible();
  await page.getByTestId('device-panel-build-cancel').click();
  await expect(confirm(page)).toHaveCount(0);
  await expect(build(page)).toHaveAttribute('data-channel', 'radio');
  await expect(page.getByTestId('device-panel-firmware')).toHaveAttribute('data-step', 'idle');
  await expect(status(page)).toHaveText('Up to date');
});

test('the switch is reachable with the keyboard and shows the focus ring', async ({ page }) => {
  await openLayout(page);
  await openDeviceSettings(page);
  await expect(toggle(page)).toBeVisible();
  await page.getByTestId('device-panel-name').focus();
  // Desk also has Wi-Fi setup and the Bluetooth keyboard before the build switch.
  const MAX_TABS = 40;
  let tabs = 0;
  while (tabs < MAX_TABS && !(await toggle(page).evaluate((el) => el === document.activeElement))) {
    await page.keyboard.press('Tab');
    tabs += 1;
  }
  expect(await toggle(page).evaluate((el) => el === document.activeElement), `build switch not reached after ${MAX_TABS} tabs`).toBe(true);
  await expect(toggle(page)).toBeFocused();
  expect(await toggle(page).evaluate((el) => getComputedStyle(el).outlineStyle)).toBe('solid');
  await page.keyboard.press('Enter');
  await expect(confirm(page)).toBeVisible();
  await expect(page.getByTestId('device-panel-build-cancel')).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(confirm(page)).toHaveCount(0);
  await expect(toggle(page)).toBeFocused();
  await expect(build(page)).toHaveAttribute('data-channel', 'radio');
});

test('a Device linked over Wi-Fi keeps its build until it is plugged in', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.getByRole('button', { name: 'Edit Stream deck by the window' }).click();
  await openDeviceSettings(page, 'mock0003');
  await expect(settingsPage(page).locator('.page-lede')).toHaveText('Connected over Wi-Fi');
  await expect(current(page)).toHaveText('Radio build');
  await expect(toggle(page)).toHaveAttribute('aria-disabled', 'true');
  await toggle(page).dispatchEvent('click');
  await expect(confirm(page)).toHaveCount(0);
  await expect(page.getByTestId('device-panel-usb-only')).toBeVisible();
});

test.describe('onboarding', () => {
  const ID = 'mock0009';
  const dialog = (page: Page) => page.getByTestId('onboarding');
  const next = (page: Page) => page.getByTestId('onboarding-next');

  async function plugIn(page: Page, channel: 'normal' | 'radio') {
    await page.goto('/?devices=1');
    await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
    await page.evaluate(({ id, channel }) => window.touchdeckDemo?.fresh(id, { version: '1.5.0', protocol: 2, channel }), { id: ID, channel });
    await expect(dialog(page)).toHaveAttribute('data-device-id', ID);
    await page.getByTestId('onboarding-name').fill('Shelf');
    await next(page).click();
    await expect(dialog(page)).toHaveAttribute('data-step', 'orientation');
    await next(page).click();
    await expect(dialog(page)).toHaveAttribute('data-step', 'layout');
    await next(page).click();
  }

  test('a Device on the radio build is offered Wi-Fi setup before it is done', async ({ page }) => {
    await plugIn(page, 'radio');
    await expect(dialog(page)).toHaveAttribute('data-step', 'wifi');
    await expect(page.getByTestId('onboarding-count')).toHaveText('Step 4 of 5');
    await expect(page.getByTestId('onboarding-wifi')).toContainText('Shelf runs the radio build. Wi-Fi and Bluetooth stay off until you set them up.');
    await expect(next(page)).toBeFocused();
    await next(page).click();
    await expect(dialog(page)).toHaveAttribute('data-step', 'done');
  });

  test('a Device on the normal build is never offered Wi-Fi setup', async ({ page }) => {
    await plugIn(page, 'normal');
    await expect(dialog(page)).toHaveAttribute('data-step', 'done');
    await expect(page.getByTestId('onboarding-wifi')).toHaveCount(0);
  });
});

const folder = process.env.TD_SHOTS;

for (const theme of ['light', 'dark'] as const) {
  test.describe(`shots ${theme}`, () => {
    test.skip(!folder, 'set TD_SHOTS to a folder to take screenshots');
    test.use({ colorScheme: theme });

    for (const device of ['Desk', 'Studio'] as const) {
      test(`the switch on ${device}`, async ({ page }) => {
        await openPanel(page, device, { starter: true });
        await toggle(page).click();
        await expect(confirm(page)).toBeVisible();
        await page.mouse.move(0, 0);
        await page.waitForTimeout(300);
        await page.screenshot({ path: join(folder!, `radio-build-${device.toLowerCase()}-${theme}.png`) });
      });
    }

    test('the onboarding Wi-Fi step', async ({ page }) => {
      await page.goto('/?devices=1');
      await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
      await page.evaluate(() => window.touchdeckDemo?.fresh('mock0009', { version: '1.5.0', protocol: 2, channel: 'radio' }));
      await page.getByTestId('onboarding-name').fill('Shelf');
      for (const step of ['orientation', 'layout', 'wifi']) {
        await page.getByTestId('onboarding-next').click();
        await expect(page.getByTestId('onboarding')).toHaveAttribute('data-step', step);
      }
      await page.mouse.move(0, 0);
      await page.waitForTimeout(300);
      await page.screenshot({ path: join(folder!, `onboarding-wifi-${theme}.png`) });
    });
  });
}
