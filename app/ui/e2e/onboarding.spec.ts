import { expect, test, type Page } from '@playwright/test';
import { openDeviceSettings } from './layout-helpers.ts';

const ID = 'mock0009';
const dialog = (page: Page) => page.getByTestId('onboarding');
const next = (page: Page) => page.getByTestId('onboarding-next');
const devices = (page: Page) => page.getByRole('list', { name: 'Devices' });
// The open dialog hides Home from the accessibility tree, so its Device is found by class.
const shelf = (page: Page) => page.locator('.device-row > .device-figure').nth(1);

interface StoredLayout {
  name: string;
  pages: { tiles?: unknown[] }[];
}

interface Record {
  layouts: { landscape: StoredLayout | null; portrait: StoredLayout | null };
  puts: number;
  settings: { orientation: string };
  reboots: number;
}

async function record(page: Page): Promise<Record> {
  return page.evaluate(async (id) => (await fetch(`/api/v1/test/devices/${id}/record`)).json(), ID);
}

async function panel(page: Page, name: string) {
  await page.getByRole('button', { name: `Edit ${name}` }).click();
  await expect(page.getByTestId('editor')).toHaveAttribute('data-device-id', ID);
  await openDeviceSettings(page, ID);
}

async function home(page: Page) {
  await page.goto('/?devices=1');
  await expect(devices(page)).toBeVisible();
}

async function plugIn(page: Page, firmware = { version: '1.5.0', protocol: 2 }) {
  await page.evaluate(({ id, firmware }) => window.touchdeckDemo?.fresh(id, firmware), { id: ID, firmware });
  await expect(dialog(page)).toHaveAttribute('data-device-id', ID);
}

async function reconnect(page: Page) {
  await page.evaluate((id) => window.touchdeckDemo?.offline(id), ID);
  await expect(shelf(page)).toContainText('Not connected');
  await page.evaluate((id) => window.touchdeckDemo?.device(id, { link: { state: 'connected', port: 'COM9', since_ms: Date.now(), protocol: 2 } }), ID);
  await expect(shelf(page).getByTestId('device-status')).toHaveText(/: Connected/);
}

async function named(page: Page, name = 'Shelf') {
  await expect(dialog(page)).toHaveAttribute('data-step', 'name');
  await page.getByTestId('onboarding-name').fill(name);
  await next(page).click();
  await expect(dialog(page)).toHaveAttribute('data-step', 'orientation');
}

async function toLayout(page: Page) {
  await named(page);
  await next(page).click();
  await expect(dialog(page)).toHaveAttribute('data-step', 'layout');
  await expect(next(page)).toHaveText('Next');
}

test('an unknown Device sets up once, and its name shows on Home and in the editor', async ({ page }) => {
  await home(page);
  await plugIn(page);
  await expect(page.getByTestId('onboarding-count')).toHaveText('Step 1 of 4');
  await expect(page.getByTestId('onboarding-name')).toBeFocused();
  await page.getByTestId('onboarding-name').fill('Shelf');
  await page.getByTestId('onboarding-name').press('Enter');
  await expect(dialog(page)).toHaveAttribute('data-step', 'orientation');
  await expect(shelf(page)).toContainText('Shelf');
  await next(page).click();
  await expect(dialog(page)).toHaveAttribute('data-step', 'layout');
  await next(page).click();
  await expect(dialog(page)).toHaveAttribute('data-step', 'done');
  await expect(dialog(page).getByRole('heading', { level: 3 })).toHaveText('Shelf is ready');
  await page.getByTestId('onboarding-done').click();
  await expect(dialog(page)).toHaveCount(0);
  await expect(shelf(page)).toContainText('Shelf');
  await reconnect(page);
  await expect(dialog(page)).toHaveCount(0);
  await page.getByRole('button', { name: 'Edit Shelf' }).click();
  await expect(page.getByTestId('editor')).toHaveAttribute('data-device-id', ID);
  await expect(page.getByTestId('device-name')).toHaveText('Shelf');
});

test('Skip leaves the Device usable, does not reopen, and the Device panel can still name it', async ({ page }) => {
  await home(page);
  await plugIn(page);
  await page.keyboard.press('Escape');
  await expect(dialog(page)).toHaveCount(0);
  await expect(shelf(page)).toContainText('Unnamed Device');
  await reconnect(page);
  await expect(dialog(page)).toHaveCount(0);
  await panel(page, 'Unnamed Device');
  const name = page.getByTestId('device-panel-name');
  await expect(name).toBeEnabled();
  await name.fill('Shelf');
  await name.press('Enter');
  await expect(page.getByTestId('settings-title')).toHaveText('Shelf');
  await page.getByRole('button', { name: 'Layout', exact: true }).click();
  await expect(page.getByTestId('device-name')).toHaveText('Shelf');
});

test('the Close button skips the same way', async ({ page }) => {
  await home(page);
  await plugIn(page);
  await page.getByTestId('onboarding-close').click();
  await expect(dialog(page)).toHaveCount(0);
  await reconnect(page);
  await expect(dialog(page)).toHaveCount(0);
});

test('a name the Device would refuse stays on the step with the reason', async ({ page }) => {
  await home(page);
  await plugIn(page);
  await page.getByTestId('onboarding-name').fill('   ');
  await next(page).click();
  await expect(page.getByTestId('onboarding-name-hint')).toHaveText('Enter a name.');
  await expect(page.getByTestId('onboarding-name')).toHaveAttribute('aria-invalid', 'true');
  await page.getByTestId('onboarding-name').fill('x'.repeat(49));
  await next(page).click();
  await expect(page.getByTestId('onboarding-name-hint')).toHaveText('Names can be up to 48 characters.');
  await expect(dialog(page)).toHaveAttribute('data-step', 'name');
});

test('Starter sends the Starter Layout, and Edit Layout opens it', async ({ page }) => {
  await home(page);
  await plugIn(page);
  await toLayout(page);
  await expect(page.getByTestId('onboarding-starter')).toHaveAttribute('data-checked', 'true');
  await expect(page.getByTestId('onboarding-keep')).toHaveCount(0);
  await next(page).click();
  await expect(dialog(page)).toHaveAttribute('data-step', 'done');
  expect((await record(page)).layouts.landscape?.name).toBe('Starter');
  await page.getByTestId('onboarding-edit').click();
  await expect(dialog(page)).toHaveCount(0);
  await expect(page.getByTestId('editor')).toHaveAttribute('data-device-id', ID);
  await expect(page.locator('[data-testid="tile"][data-id="playpause"]')).toBeVisible();
});

test('Blank sends one empty Page', async ({ page }) => {
  await home(page);
  await plugIn(page);
  await toLayout(page);
  await page.getByTestId('onboarding-blank').click();
  await next(page).click();
  await expect(dialog(page)).toHaveAttribute('data-step', 'done');
  const layout = (await record(page)).layouts.landscape;
  expect(layout?.name).toBe('New Layout');
  expect(layout?.pages).toHaveLength(1);
  expect(layout?.pages[0].tiles ?? []).toHaveLength(0);
});

test('Keep leaves the Layout already on the Device', async ({ page }) => {
  await home(page);
  await page.evaluate((id) => window.touchdeckDemo?.push(id, ['work']), ID);
  await plugIn(page);
  await toLayout(page);
  const keep = page.getByTestId('onboarding-keep');
  await expect(keep).toHaveAttribute('data-checked', 'true');
  await expect(keep).toContainText('Keep “Demo Layout”');
  await expect(keep).toContainText('The Layout on the Device now: 1 Page, 8 Tiles.');
  const before = (await record(page)).puts;
  await next(page).click();
  await expect(dialog(page)).toHaveAttribute('data-step', 'done');
  const after = await record(page);
  expect(after.puts).toBe(before);
  expect(after.layouts.landscape?.name).toBe('Demo Layout');
});

test('a new orientation restarts the Device before the Layout step', async ({ page }) => {
  await home(page);
  await plugIn(page);
  await named(page);
  await page.getByTestId('onboarding-portrait').click();
  await next(page).click();
  await expect(next(page)).toHaveText('Restarting…');
  await expect(dialog(page)).toHaveAttribute('data-step', 'layout', { timeout: 5_000 });
  expect(await record(page)).toMatchObject({ reboots: 1, settings: { orientation: 'portrait' } });
  await expect(next(page)).toHaveText('Next');
  await next(page).click();
  await expect(dialog(page)).toHaveAttribute('data-step', 'done');
  expect((await record(page)).layouts.portrait?.name).toBe('Starter');
});

test.describe('the Firmware step', () => {
  const older = { version: '1.4.0', protocol: 2 };

  async function toFirmware(page: Page) {
    await home(page);
    await plugIn(page, older);
    await expect(page.getByTestId('onboarding-count')).toHaveText('Step 1 of 5');
    await toLayout(page);
    await next(page).click();
    await expect(dialog(page)).toHaveAttribute('data-step', 'firmware');
    await expect(dialog(page)).toContainText('Shelf runs Firmware 1.4.0. Firmware 1.5.0 adds newer Tiles.');
  }

  test('updates the Device', async ({ page }) => {
    await toFirmware(page);
    await page.getByTestId('onboarding-update').click();
    await expect(page.getByTestId('onboarding-firmware-status')).toHaveText('Updated to 1.5.0', { timeout: 10_000 });
    await next(page).click();
    await page.getByTestId('onboarding-done').click();
    await panel(page, 'Shelf');
    await expect(page.getByTestId('device-panel-firmware')).toContainText('Firmware 1.5.0');
    await expect(page.getByTestId('device-panel-update')).toHaveCount(0);
  });

  test('can be skipped', async ({ page }) => {
    await toFirmware(page);
    await page.getByTestId('onboarding-skip').click();
    await expect(dialog(page)).toHaveAttribute('data-step', 'done');
    await page.getByTestId('onboarding-done').click();
    await panel(page, 'Shelf');
    await expect(page.getByTestId('device-panel-firmware')).toContainText('Firmware 1.4.0');
    await expect(page.getByTestId('device-panel-update')).toBeVisible();
  });

  test('a rollback says the Device still works, and Skip still moves on', async ({ page }) => {
    await toFirmware(page);
    await page.evaluate((id) => fetch(`/api/v1/test/devices/${id}/mock`, { method: 'POST', body: JSON.stringify({ action: 'rollback' }) }), ID);
    await page.getByTestId('onboarding-update').click();
    const status = page.getByTestId('onboarding-firmware-status');
    await expect(status).toHaveAttribute('data-code', 'firmware.rolled_back', { timeout: 10_000 });
    await expect(status).toContainText("Couldn't update. Shelf went back to 1.4.0 and still works.");
    await expect(page.getByTestId('onboarding-update')).toHaveText('Retry');
    await page.getByTestId('onboarding-skip').click();
    await expect(dialog(page)).toHaveAttribute('data-step', 'done');
  });

  test('a v1 Device updates first, then takes its name', async ({ page }) => {
    await home(page);
    await plugIn(page, { version: '1.2.0', protocol: 1 });
    await expect(dialog(page)).toHaveAttribute('data-step', 'firmware');
    await expect(page.getByTestId('onboarding-count')).toHaveText('Step 1 of 5');
    await expect(page.getByTestId('onboarding-back')).toHaveCount(0);
    await page.getByTestId('onboarding-update').click();
    await expect(page.getByTestId('onboarding-firmware-status')).toHaveText('Updated to 1.5.0', { timeout: 10_000 });
    await next(page).click();
    await named(page);
    await expect(shelf(page)).toContainText('Shelf');
  });

  test('a skipped v1 Device says naming needs newer Firmware and moves on', async ({ page }) => {
    await home(page);
    await plugIn(page, { version: '1.2.0', protocol: 1 });
    await page.getByTestId('onboarding-skip').click();
    await expect(dialog(page)).toHaveAttribute('data-step', 'name');
    await page.getByTestId('onboarding-name').fill('Shelf');
    await next(page).click();
    await expect(page.getByTestId('onboarding-name')).toBeDisabled();
    await expect(page.getByTestId('onboarding-name-hint')).toContainText('Firmware');
    await next(page).click();
    await expect(dialog(page)).toHaveAttribute('data-step', 'orientation');
    await expect(shelf(page)).toContainText('Unnamed Device');
  });
});

test('focus is visible on the name and on each choice', async ({ page }) => {
  await home(page);
  await plugIn(page);
  const input = page.getByTestId('onboarding-name');
  await expect(input).toBeFocused();
  await page.keyboard.type('Shelf');
  expect(await input.evaluate((el) => getComputedStyle(el).outlineStyle)).toBe('solid');
  await page.keyboard.press('Enter');
  await expect(dialog(page)).toHaveAttribute('data-step', 'orientation');
  const landscape = page.getByTestId('onboarding-landscape');
  await expect(landscape.locator('input')).toBeFocused();
  await expect.poll(() => landscape.evaluate((el) => getComputedStyle(el).outlineStyle)).toBe('solid');
  await page.keyboard.press('ArrowRight');
  const portrait = page.getByTestId('onboarding-portrait');
  await expect(portrait).toHaveAttribute('data-checked', 'true');
  await expect.poll(() => portrait.evaluate((el) => getComputedStyle(el).outlineStyle)).toBe('solid');
  expect(await landscape.evaluate((el) => getComputedStyle(el).outlineStyle)).toBe('none');
});
