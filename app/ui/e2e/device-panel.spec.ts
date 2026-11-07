import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { openLayout } from './layout-helpers.ts';

const panel = (page: Page) => page.getByTestId('device-panel');
const status = (page: Page) => page.getByTestId('device-panel-status');

interface Record {
  settings: {
    orientation: string;
    brightness: number | null;
    photo_frame: { enabled: boolean; timeout_s: number; interval_s: number; folder: string | null } | null;
  };
  reboots: number;
}

const PICKED = 'D:\\Pictures\\Desk frame';

async function photoFrameOn(page: Page) {
  const photo = page.getByTestId('device-panel-photo-frame');
  await expect(photo).toBeEnabled();
  await photo.click();
  await expect(page.getByTestId('device-panel-photo-frame-after')).toHaveValue('60');
}

async function record(page: Page, device = 'mock0001'): Promise<Record> {
  return page.evaluate(async (id) => (await fetch(`/api/v1/test/devices/${id}/record`)).json(), device);
}

async function openPanel(page: Page, path = '/') {
  await openLayout(page, path);
  await page.getByTestId('device-name').click();
  await expect(panel(page)).toBeVisible();
  await expect(page.getByTestId('device-panel-name')).toBeEnabled();
}

async function studio(page: Page, before = async () => {}) {
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.getByRole('button', { name: 'Edit Studio' }).click();
  await expect(page.getByTestId('editor')).toHaveAttribute('data-device-id', 'mock0002');
  await page.evaluate(() => document.fonts.ready);
  await before();
  await page.getByTestId('device-name').click();
  await expect(page.getByTestId('device-panel-update')).toBeVisible();
}

test('renames the Device, and the new name shows in the header and on Home', async ({ page }) => {
  await openPanel(page);
  await expect(page.getByTestId('device-panel-name-hint')).toHaveText('Stored on Desk.');
  const name = page.getByTestId('device-panel-name');
  await name.fill('Desk left');
  await name.press('Enter');
  await expect(status(page)).toHaveAttribute('data-event', 'saved');
  await expect(page.getByTestId('device-name')).toHaveText('Desk left');
  await expect(panel(page).locator('.popover-title')).toHaveText('Desk left');
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Home', exact: true }).click();
  await expect(page.getByRole('list', { name: 'Devices' }).getByRole('listitem').first()).toContainText('Desk left');
});

test('refuses a blank name with the rule and keeps the old one', async ({ page }) => {
  await openPanel(page);
  const name = page.getByTestId('device-panel-name');
  await name.fill('   ');
  await name.press('Enter');
  await expect(page.getByTestId('device-panel-name-hint')).toHaveText('Give the Device a name.');
  await expect(page.getByTestId('device-panel-name')).toHaveAttribute('aria-invalid', 'true');
  await expect(page.getByTestId('device-name')).toHaveText('Desk');
});

test('refuses a name with an invisible character and keeps the old one', async ({ page }) => {
  await openPanel(page);
  const name = page.getByTestId('device-panel-name');
  for (const hidden of ['Desk‮PC', 'Deskㅤ']) {
    await name.fill(hidden);
    await name.press('Enter');
    await expect(page.getByTestId('device-panel-name-hint')).toHaveText('Remove tabs, line breaks and other special characters.');
    await expect(name).toHaveAttribute('aria-invalid', 'true');
    await expect(page.getByTestId('device-name')).toHaveText('Desk');
  }
});

test('switches orientation after the restart confirm, and the Device restarts', async ({ page }) => {
  await openPanel(page);
  await page.getByTestId('device-panel-portrait').click();
  const confirm = page.getByTestId('device-panel-confirm');
  await expect(confirm).toContainText('Restart Desk to switch to Portrait?');
  await expect(confirm).toContainText('Each orientation keeps its own Layout.');
  await page.getByTestId('device-panel-confirm-restart').click();
  await expect(status(page)).toHaveAttribute('data-event', 'restarting');
  await expect(status(page)).toHaveText('Restarting…');
  await expect(page.getByTestId('connection')).toHaveText('Connected');
  await expect(status(page)).not.toHaveAttribute('data-event', 'restarting', { timeout: 5_000 });
  const after = await record(page);
  expect(after.settings.orientation).toBe('portrait');
  expect(after.reboots).toBe(1);
  await expect(page.getByTestId('device-panel-portrait')).toHaveAttribute('aria-pressed', 'true');
});

test('Cancel keeps the orientation and restarts nothing', async ({ page }) => {
  await openPanel(page);
  await page.getByTestId('device-panel-portrait').click();
  await page.getByTestId('device-panel-confirm-cancel').click();
  await expect(page.getByTestId('device-panel-confirm')).toHaveCount(0);
  await expect(page.getByTestId('device-panel-landscape')).toHaveAttribute('aria-pressed', 'true');
  const after = await record(page);
  expect(after.settings.orientation).toBe('landscape');
  expect(after.reboots).toBe(0);
});

test('sets brightness from the slider and shows the value', async ({ page }) => {
  await openPanel(page);
  await expect(page.getByTestId('device-panel-brightness-value')).toHaveText('80%');
  const thumb = page.getByTestId('device-panel-brightness').getByRole('slider');
  await thumb.focus();
  for (let i = 0; i < 5; i++) await page.keyboard.press('ArrowLeft');
  await expect(page.getByTestId('device-panel-brightness-value')).toHaveText('75%');
  await expect.poll(async () => (await record(page)).settings.brightness).toBe(75);
});

test('Restart restarts the Device and its status comes back', async ({ page }) => {
  await openPanel(page);
  await page.getByTestId('device-panel-restart').click();
  await expect(status(page)).toHaveText('Restarting…');
  await expect(page.getByTestId('connection')).toHaveText('Connecting…');
  await expect(page.getByTestId('connection')).toHaveText('Connected');
  await expect(status(page)).toHaveText('');
  expect((await record(page)).reboots).toBe(1);
});

test('updates the Firmware with progress, then shows the new version', async ({ page }) => {
  await studio(page, async () => {
    await page.evaluate(() => window.touchdeckDemo?.firmware.hold('mock0002'));
  });
  const firmware = page.getByTestId('device-panel-firmware');
  const progress = page.getByTestId('device-panel-firmware-status');
  await expect(firmware).toContainText('Firmware 1.4.0');
  await expect(progress).toHaveText('1.5.0 available');
  await page.getByTestId('device-panel-update').click();
  await expect(progress).toHaveText('Updating… 40%');
  await page.evaluate(() => {
    window.touchdeckDemo?.firmware.release('mock0002');
    window.touchdeckDemo?.firmware.hold('mock0002', 'rebooting');
  });
  await expect(progress).toHaveText('Restarting…');
  await page.evaluate(() => window.touchdeckDemo?.firmware.release('mock0002'));
  await expect(progress).toHaveText('Updated to 1.5.0', { timeout: 10_000 });
  await expect(firmware).toContainText('Firmware 1.5.0');
  await expect(page.getByTestId('device-panel-update')).toHaveCount(0);
  await expect(page.getByTestId('connection')).toHaveText('Connected');
});

test('an update that rolls back says so, and keeps the old version', async ({ page }) => {
  await studio(page);
  await page.evaluate(async () => {
    await fetch('/api/v1/test/devices/mock0002/mock', { method: 'POST', body: JSON.stringify({ action: 'rollback' }) });
  });
  await page.getByTestId('device-panel-update').click();
  const error = page.getByTestId('device-panel-firmware-error');
  await expect(error).toHaveAttribute('data-code', 'firmware.rolled_back', { timeout: 10_000 });
  await expect(error.locator('.panel-status-text')).toHaveText(
    "The new Firmware didn't start, so Studio went back to 1.4.0. It still works. Retry, or repair it from Troubleshooting.",
  );
  await error.getByText('Details').click();
  await expect(error).toContainText('Failed: The Device rolled back to its previous Firmware.');
  await expect(page.getByTestId('device-panel-firmware')).toContainText('Firmware 1.4.0');
  await expect(page.getByTestId('device-panel-update')).toBeVisible();
});

test('a cancelled update says it stopped, keeps the old version, and offers Update again', async ({ page }) => {
  await studio(page, async () => {
    await page.evaluate(() => window.touchdeckDemo?.firmware.hold('mock0002'));
  });
  await page.getByTestId('device-panel-update').click();
  await expect(page.getByTestId('device-panel-firmware-status')).toHaveText('Updating… 40%');
  await page.evaluate(() => window.touchdeckDemo?.firmware.cancel('mock0002'));
  const stopped = page.getByTestId('device-panel-firmware-stopped');
  await expect(stopped.locator('.panel-status-text')).toHaveText('Update stopped. Studio still runs 1.4.0.');
  await expect(page.getByTestId('device-panel-firmware-error')).toHaveCount(0);
  await expect(page.getByTestId('device-panel-firmware')).toContainText('Firmware 1.4.0');
  await expect(page.getByTestId('device-panel-update')).toBeVisible();
  await page.evaluate(() => window.touchdeckDemo?.firmware.release('mock0002'));
});

test("a v1 Device's name and brightness say they need newer Firmware", async ({ page }) => {
  await openLayout(page);
  await page.evaluate(() => {
    window.touchdeckDemo?.device('mock0001', { firmware: { version: '1.2.0', protocol: 1, build: 'old', recovery: false, update_available: null, channel: 'normal' } });
  });
  await page.getByTestId('device-name').click();
  await expect(page.getByTestId('device-panel-name-hint')).toHaveText("Desk's Firmware can't change this yet.");
  await expect(page.getByTestId('device-panel-name')).toBeDisabled();
  await expect(page.getByTestId('device-panel-landscape')).toBeEnabled();
});

test('schema 1 Tiles carry no Needs Firmware badge, even on v1 Firmware', async ({ page }) => {
  await openLayout(page);
  await page.getByTestId('start-starter').click();
  await page.evaluate(() => {
    window.touchdeckDemo?.device('mock0001', { firmware: { version: '1.2.0', protocol: 1, build: 'old', recovery: false, update_available: null, channel: 'normal' }, capabilities: null });
  });
  await expect(page.getByTestId('tile').first()).toBeVisible();
  await expect(page.getByTestId('tile-needs')).toHaveCount(0);
});

test("brightness on Firmware that can't set it explains why", async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Edit Stream deck by the window' }).click();
  await page.getByTestId('device-name').click();
  await expect(page.getByTestId('device-panel-brightness-hint')).toHaveText("Stream deck by the window's Firmware can't change this yet.");
  await expect(page.getByTestId('device-panel-brightness')).toHaveCount(0);
});

test('turns the photo frame on, sets when it starts and picks its folder', async ({ page }) => {
  await openPanel(page);
  const photo = page.getByTestId('device-panel-photo-frame');
  await expect(page.getByTestId('device-panel-photo-frame-hint')).toHaveText("Pictures from this PC when Desk sits idle, or a clock when Open TouchDeck isn't running.");
  await expect(photo).toHaveAttribute('aria-checked', 'false');
  await expect(page.getByTestId('device-panel-photo-frame-after')).toHaveCount(0);
  await photoFrameOn(page);
  await expect(status(page)).toHaveAttribute('data-event', 'saved');
  await expect(photo).toHaveAttribute('aria-checked', 'true');
  await page.getByTestId('device-panel-photo-frame-after').selectOption({ label: '5 min' });
  await expect.poll(async () => (await record(page)).settings.photo_frame?.timeout_s).toBe(300);

  await expect(page.getByTestId('device-panel-folder')).toHaveText('No folder chosen');
  await page.evaluate(() => window.touchdeckDemo?.pickFolder(null));
  await page.getByTestId('device-panel-folder-choose').click();
  await expect(page.getByTestId('device-panel-folder')).toHaveText('No folder chosen');
  await page.evaluate((folder) => window.touchdeckDemo?.pickFolder(folder), PICKED);
  await page.getByTestId('device-panel-folder-choose').click();
  await expect(page.getByTestId('device-panel-folder')).toHaveText(PICKED);
  await expect(page.getByTestId('device-panel-folder-hint')).toHaveText('Nothing is stored on Desk.');
  expect((await record(page)).settings.photo_frame).toEqual({ enabled: true, timeout_s: 300, interval_s: 15, folder: PICKED });

  await photo.click();
  await expect(page.getByTestId('device-panel-photo-frame-after')).toHaveCount(0);
  await expect.poll(async () => (await record(page)).settings.photo_frame?.enabled).toBe(false);
});

test('in a browser the folder shows as text, and choosing one is left to the app', async ({ page }) => {
  await openPanel(page, '/?shell=browser');
  await photoFrameOn(page);
  await expect(page.getByTestId('device-panel-folder')).toHaveText('No folder chosen');
  await expect(page.getByTestId('device-panel-folder-choose')).toHaveCount(0);
  await expect(page.getByTestId('device-panel-folder-hint')).toHaveText('Choose a folder in the Open TouchDeck app.');
});

test("the photo frame on Firmware without it explains why", async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Edit Stream deck by the window' }).click();
  await page.getByTestId('device-name').click();
  await expect(page.getByTestId('device-panel-photo-frame-hint')).toHaveText("Stream deck by the window's Firmware can't change this yet.");
  await expect(page.getByTestId('device-panel-photo-frame')).toHaveCount(0);
});

test('Enter opens the panel with focus on the first field, and Escape returns focus to the name', async ({ page }) => {
  await openLayout(page);
  const trigger = page.getByTestId('device-name');
  await trigger.focus();
  await page.keyboard.press('Enter');
  await expect(panel(page)).toBeVisible();
  await expect(page.getByTestId('device-panel-name')).toBeFocused();
  await expect(trigger).toHaveAttribute('aria-expanded', 'true');
  const ring = await page.getByTestId('device-panel-name').evaluate((el) => getComputedStyle(el).outlineStyle);
  expect(ring).toBe('solid');
  await page.keyboard.press('Tab');
  await expect(page.getByTestId('device-panel-landscape')).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(panel(page)).toHaveCount(0);
  await expect(trigger).toBeFocused();
  await page.keyboard.press('Space');
  await expect(panel(page)).toBeVisible();
});

test('every control in the panel is reachable with Tab and shows the focus ring', async ({ page }) => {
  await openLayout(page);
  await page.getByTestId('device-name').focus();
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('device-panel-brightness-value')).toHaveText('80%');
  await expect(page.getByTestId('device-panel-wifi-setup-status')).toHaveAttribute('data-state', 'off');
  const reached: string[] = [];
  for (let i = 0; i < 9; i++) {
    const focused = await page.evaluate(() => {
      const el = document.activeElement;
      if (!(el instanceof HTMLElement)) return null;
      const ringed = el.closest('.slider-thumb') ?? el;
      return { id: el.dataset.testid ?? el.closest('[data-testid]')?.getAttribute('data-testid') ?? el.tagName, ring: getComputedStyle(ringed).outlineStyle };
    });
    if (focused) {
      reached.push(focused.id);
      expect(focused.ring, focused.id).toBe('solid');
    }
    await page.keyboard.press('Tab');
  }
  expect(reached).toEqual([
    'device-panel-name',
    'device-panel-landscape',
    'device-panel-brightness',
    'device-panel-photo-frame',
    'device-panel-wifi-setup-ssid',
    'device-panel-wifi-setup-password',
    'device-panel-wifi-setup-join',
    'device-panel-wifi-begin',
    'device-panel-restart',
  ]);
});

const folder = process.env.TD_SHOTS;

for (const theme of ['light', 'dark'] as const) {
  test.describe(`shots ${theme}`, () => {
    test.skip(!folder, 'set TD_SHOTS to a folder to take screenshots');
    test.use({ colorScheme: theme });

    test('the open panel', async ({ page }) => {
      await openLayout(page);
      await page.getByTestId('start-starter').click();
      await page.getByTestId('device-name').click();
      await expect(page.getByTestId('device-panel-brightness-value')).toHaveText('80%');
      await page.mouse.move(0, 0);
      await page.waitForTimeout(300);
      await page.screenshot({ path: join(folder!, `device-panel-${theme}.png`) });
    });

    test('an update in progress', async ({ page }) => {
      await studio(page, async () => {
        await page.getByTestId('start-starter').click();
        await page.evaluate(() => window.touchdeckDemo?.firmware.hold('mock0002'));
      });
      await page.getByTestId('device-panel-update').click();
      await expect(page.getByTestId('device-panel-firmware-status')).toHaveText('Updating… 40%');
      await page.mouse.move(0, 0);
      await page.waitForTimeout(300);
      await page.screenshot({ path: join(folder!, `device-panel-updating-${theme}.png`) });
      await page.evaluate(() => window.touchdeckDemo?.firmware.release('mock0002'));
    });

    test('the photo frame, on with a folder', async ({ page }) => {
      await openLayout(page);
      await page.getByTestId('start-starter').click();
      await page.getByTestId('device-name').click();
      await photoFrameOn(page);
      await page.getByTestId('device-panel-folder-choose').click();
      await expect(page.getByTestId('device-panel-folder')).toHaveText(PICKED);
      await page.mouse.move(0, 0);
      await page.waitForTimeout(300);
      await page.screenshot({ path: join(folder!, `device-panel-photo-frame-${theme}.png`) });
    });

    test('the photo frame in a browser', async ({ page }) => {
      await openLayout(page, '/?shell=browser');
      await page.getByTestId('start-starter').click();
      await page.getByTestId('device-name').click();
      await photoFrameOn(page);
      await page.mouse.move(0, 0);
      await page.waitForTimeout(300);
      await page.screenshot({ path: join(folder!, `device-panel-photo-frame-browser-${theme}.png`) });
    });

    test('focus on the Device name', async ({ page }) => {
      await openLayout(page);
      await page.getByTestId('start-starter').click();
      await page.getByTestId('device-name').focus();
      await page.keyboard.press('Shift+Tab');
      await page.keyboard.press('Tab');
      await page.waitForTimeout(200);
      await page.screenshot({ path: join(folder!, `focus-devicename-${theme}.png`) });
    });
  });
}
