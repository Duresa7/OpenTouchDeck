import { expect, test, type Page } from '@playwright/test';

const nav = (page: Page) => page.getByRole('navigation', { name: 'Main' });
const settings = (page: Page) => page.getByRole('dialog', { name: 'Settings' });
const help = (page: Page) => page.getByRole('dialog', { name: 'Troubleshooting' });
const confirmation = (page: Page) => page.getByRole('alertdialog');
const startSwitch = (page: Page) => settings(page).getByRole('switch', { name: 'Start at sign-in' });
const browsers = (page: Page) => settings(page).getByTestId('paired-browser');
const topics = (page: Page) => page.evaluate(() => window.touchdeckDemo?.topics() ?? []);

async function start(page: Page, query = '') {
  await page.goto(`/${query}`);
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
}

async function openSettings(page: Page) {
  await nav(page).getByRole('button', { name: 'Settings' }).click();
  await expect(startSwitch(page)).toBeEnabled();
}

async function openTroubleshooting(page: Page) {
  await openSettings(page);
  await settings(page).getByRole('button', { name: /^Troubleshooting/ }).click();
  await expect(help(page)).toBeVisible();
}

test.describe('Start at sign-in', () => {
  test('toggles and is still set when Settings opens again', async ({ page }) => {
    await start(page);
    await openSettings(page);
    await expect(startSwitch(page)).not.toBeChecked();
    await expect(settings(page)).toContainText('Runs in the tray when you sign in to Windows, so your Devices work right away.');

    await startSwitch(page).click();
    await expect(startSwitch(page)).toBeChecked();
    await page.keyboard.press('Escape');
    await expect(settings(page)).toBeHidden();

    await openSettings(page);
    await expect(startSwitch(page)).toBeChecked();
    await startSwitch(page).click();
    await page.keyboard.press('Escape');
    await openSettings(page);
    await expect(startSwitch(page)).not.toBeChecked();
  });
});

test.describe('Paired browsers', () => {
  test('lists each browser with its last use, and Forget asks first, then removes it for good', async ({ page }) => {
    await start(page);
    await openSettings(page);
    await expect(browsers(page)).toHaveCount(2);
    await expect(browsers(page).first()).toContainText('Edge on Windows · paired');
    await expect(browsers(page).first()).toContainText('Last used');

    await settings(page).getByRole('button', { name: 'Forget Edge on Windows' }).click();
    await expect(confirmation(page)).toHaveAccessibleName('Forget this browser?');
    await expect(confirmation(page)).toContainText('It needs a new code to use Open TouchDeck again.');
    await confirmation(page).getByRole('button', { name: 'Cancel' }).click();
    await expect(confirmation(page)).toBeHidden();
    await expect(browsers(page)).toHaveCount(2);

    await settings(page).getByRole('button', { name: 'Forget Edge on Windows' }).click();
    await confirmation(page).getByRole('button', { name: 'Forget', exact: true }).click();
    await expect(browsers(page)).toHaveCount(1);
    await expect(browsers(page)).not.toContainText('Edge on Windows');
    await expect(settings(page).getByRole('button', { name: 'Forget all' })).toHaveCount(0);

    await page.keyboard.press('Escape');
    await openSettings(page);
    await expect(browsers(page)).toHaveCount(1);
    await expect(browsers(page)).toContainText('Firefox on Windows');
  });

  test('Forget all removes every browser and shows how to pair one', async ({ page }) => {
    await start(page);
    await openSettings(page);
    await settings(page).getByRole('button', { name: 'Forget all' }).click();
    await expect(confirmation(page)).toHaveAccessibleName('Forget all browsers?');
    await confirmation(page).getByRole('button', { name: 'Forget all' }).click();
    await expect(browsers(page)).toHaveCount(0);
    await expect(settings(page)).toContainText('No browsers paired yet.');
  });

  test('Reset pairing confirms first, then signs out every browser while this window carries on', async ({ page }) => {
    await start(page);
    await openSettings(page);
    await settings(page).getByRole('button', { name: 'Reset', exact: true }).click();
    await expect(confirmation(page)).toHaveAccessibleName('Reset pairing?');
    await expect(confirmation(page)).toContainText('Every window and browser is signed out.');
    await page.keyboard.press('Escape');
    await expect(confirmation(page)).toBeHidden();
    await expect(settings(page)).toBeVisible();
    await expect(browsers(page)).toHaveCount(2);

    await settings(page).getByRole('button', { name: 'Reset', exact: true }).click();
    await confirmation(page).getByRole('button', { name: 'Reset', exact: true }).click();
    await expect(browsers(page)).toHaveCount(0);
    await expect(settings(page)).toContainText('No browsers paired yet.');
    await expect(settings(page)).not.toContainText("Couldn't");

    await page.keyboard.press('Escape');
    await expect(page.getByRole('listitem')).toHaveCount(3);
    await openSettings(page);
    await expect(settings(page)).toContainText('Open TouchDeck 0.2.0');
  });

  test('the desktop window offers to open a browser here, and a browser does not', async ({ page }) => {
    await start(page);
    await openSettings(page);
    await expect(settings(page).getByTestId('open-browser-row')).toHaveCount(0);
    await page.addInitScript(() => {
      window.openTouchDeck = { version: 1, session: async () => 'demo', onSessionChanged: () => {} };
    });
    await start(page);
    await expect(page.locator('.open-browser')).toHaveCount(0);
    await openSettings(page);
    const row = settings(page).getByTestId('open-browser-row');
    await expect(row).toContainText('Open in a browser');
    await expect(row).toContainText('Pairs a browser on this PC.');
    await expect(row.getByRole('button', { name: 'Open' })).toBeVisible();
  });

  test('Icon packs and Brandfetch logos moved to Integrations', async ({ page }) => {
    await start(page);
    await openSettings(page);
    await expect(settings(page)).not.toContainText('Icon packs');
    await expect(settings(page)).not.toContainText('Brandfetch');
  });

  test('with none paired it says how to pair one', async ({ page }) => {
    await start(page, '?browsers=0');
    await openSettings(page);
    await expect(settings(page)).toContainText('No browsers paired yet.');
    await expect(settings(page).getByRole('button', { name: 'Forget all' })).toHaveCount(0);
  });
});

test.describe('Troubleshooting', () => {
  test('shows each Device with its link and Firmware fields, and Back returns to Settings', async ({ page }) => {
    await start(page);
    await openTroubleshooting(page);
    await expect(help(page)).toContainText('Device details, and tools to check that taps reach this PC.');
    const details = help(page).getByTestId('device-details');
    await expect(details).toHaveCount(3);
    for (const [i, name] of ['Desk', 'Studio', 'Stream deck by the window'].entries()) {
      await expect(details.nth(i)).toContainText(`${name} is the demo Device.`);
      await expect(details.nth(i)).toContainText('Connected');
    }
    const desk = details.first();
    for (const label of ['Product', 'Protocol', 'Firmware build', 'Firmware version', 'Running from', 'Sends keys through', 'Safe mode', 'Current Layout']) {
      await expect(desk.locator('dt', { hasText: label })).toHaveCount(1);
    }
    await expect(desk.locator('[data-fact="proto"] dd')).toHaveText('2');
    await expect(desk.locator('[data-fact="version"] dd')).toHaveText('1.5.0');
    await expect(desk.locator('[data-fact="elf_sha256"] dd')).toHaveText('fake');
    await expect(desk.locator('[data-fact="recovery"] dd')).toHaveText('No');
    await expect(desk.locator('[data-fact="product"] dd')).toHaveText('touchdeck');

    await page.evaluate(() => window.touchdeckDemo?.offline('mock0001'));
    await expect(desk).toContainText('Lost Desk. Check the USB cable. It reconnects on its own.');
    await expect(desk).toContainText('Not connected');
    await expect(desk.locator('dl')).toHaveCount(0);

    await help(page).getByRole('button', { name: 'Back' }).click();
    await expect(settings(page)).toBeVisible();
    await expect(settings(page).getByRole('button', { name: /^Troubleshooting/ })).toBeFocused();
  });

  test('Connection help on an empty Home opens Troubleshooting, and the nav opens Settings again', async ({ page }) => {
    await page.goto('/?devices=0');
    await page.getByRole('button', { name: 'Connection help' }).click();
    await expect(help(page)).toContainText('No Device connected. Plug one in with a USB cable.');
    await expect(help(page).getByRole('heading', { name: 'Firmware console' })).toHaveCount(0);
    await page.keyboard.press('Escape');
    await nav(page).getByRole('button', { name: 'Settings' }).click();
    await expect(settings(page)).toBeVisible();
  });

  test('a console line round-trips, and the console listens only while it is open', async ({ page }) => {
    await start(page);
    expect(await topics(page)).toEqual([]);
    await openTroubleshooting(page);
    await expect.poll(() => topics(page)).toEqual(['console/mock0001']);
    await expect(help(page)).toContainText("Send a command to Desk's console, like version or layout state.");

    const field = help(page).getByRole('textbox', { name: 'Console command' });
    const log = help(page).getByTestId('console-log');
    await field.fill('version');
    await help(page).getByRole('button', { name: 'Send' }).click();
    await expect(log.locator('[data-kind="sent"]')).toHaveText(['> version']);
    await expect(log.locator('[data-kind="line"]')).toHaveCount(1);
    await expect(log.locator('[data-kind="line"]')).toContainText('"cmd":"version"');
    await expect(field).toHaveValue('');

    await help(page).getByRole('group', { name: 'Device' }).getByRole('button', { name: 'Studio' }).click();
    await expect.poll(() => topics(page)).toEqual(['console/mock0002']);
    await expect(log).toContainText('Nothing yet. Send a command below.');

    await help(page).getByRole('button', { name: 'Back' }).click();
    await expect.poll(() => topics(page)).toEqual([]);
  });

  test('a line off the allowlist is not sent, and the console lists what it takes', async ({ page }) => {
    await start(page);
    await openTroubleshooting(page);
    const field = help(page).getByRole('textbox', { name: 'Console command' });
    await field.fill('reboot');
    await field.press('Enter');
    const notice = help(page).getByTestId('console-forbidden');
    await expect(notice).toContainText('“reboot” wasn\'t sent. The console takes only these read-only commands:');
    await expect(notice.getByRole('listitem')).toHaveCount(23);
    await expect(notice).toContainText('layout state');
    await expect(help(page).getByTestId('console-log').locator('[data-kind]')).toHaveCount(0);
    await expect(field).toHaveValue('reboot');

    await field.fill('heap');
    await field.press('Enter');
    await expect(notice).toBeHidden();
    await expect(help(page).getByTestId('console-log').locator('[data-kind="line"]')).toContainText('free_heap');
  });

  test('Update from a file updates a Device from the chosen file, and a file that is not Firmware says so', async ({ page }) => {
    await start(page);
    await openTroubleshooting(page);
    await expect(help(page).getByRole('heading', { name: 'Update from a file' })).toBeVisible();
    const rows = help(page).getByTestId('firmware-file');
    await expect(rows).toHaveCount(3);
    const studio = rows.filter({ hasText: 'Studio' });
    const status = studio.getByTestId('firmware-file-status');
    const choose = studio.getByRole('button', { name: 'Choose file' });
    await expect(status).toHaveText('A .tdfw file from an Open TouchDeck release.');

    await page.evaluate(() => window.touchdeckDemo?.firmware.hold('mock0002'));
    await choose.click();
    await expect(status).toHaveText('Updating… 40%');
    await expect(studio.getByRole('progressbar', { name: 'Updating Studio' })).toHaveAttribute('aria-valuenow', '40');
    await expect(choose).toHaveAttribute('aria-disabled', 'true');
    await page.evaluate(() => window.touchdeckDemo?.firmware.release('mock0002'));
    await expect(studio).toHaveAttribute('data-step', 'done');
    await expect(status).toHaveText('Updated to 1.5.0');
    await expect(help(page).getByTestId('device-details').nth(1).locator('[data-fact="version"] dd')).toHaveText('1.5.0');

    await page.evaluate(() => window.touchdeckDemo?.firmware.picks('not-firmware'));
    await choose.click();
    await expect(studio).toHaveAttribute('data-step', 'failed');
    await expect(status).toHaveText("That file isn't a Firmware file for this Device.");
    await studio.locator('summary', { hasText: 'Details' }).click();
    await expect(studio.locator('.row-details p')).toHaveText('Failed: The Firmware bundle is invalid or unavailable.');
  });
});

test.describe('About', () => {
  test('shows the version and build, and Check for updates shows Checking…, then Up to date', async ({ page }) => {
    await start(page, '?update=none');
    await expect.poll(() => page.evaluate(() => window.touchdeckDemo?.updates.calls.check ?? 0)).toBe(1);
    await page.evaluate(() => window.touchdeckDemo?.updates.script({ stepMs: 400 }));
    await openSettings(page);
    await expect(settings(page)).toContainText('Open TouchDeck 0.2.0');
    await expect(settings(page)).toContainText('Build demo');
    await settings(page).getByRole('button', { name: 'Check for updates' }).click();
    await expect(settings(page)).toContainText('Checking…');
    await expect(settings(page)).toContainText('Up to date');
  });
});

test.describe('keyboard', () => {
  test('Tab reaches every Settings control with the visible focus ring', async ({ page }) => {
    await start(page);
    await nav(page).getByRole('button', { name: 'Settings' }).focus();
    await page.keyboard.press('Enter');
    await expect(startSwitch(page)).toBeEnabled();
    await expect(browsers(page)).toHaveCount(2);
    const reached: string[] = [];
    for (let i = 0; i < 16; i++) {
      const focused = await page.evaluate(() => {
        const el = document.activeElement;
        if (!(el instanceof HTMLElement) || !el.closest('[role="dialog"]')) return null;
        const style = getComputedStyle(el);
        const labelledBy = el.getAttribute('aria-labelledby');
        const name = el.getAttribute('aria-label') ?? (labelledBy ? document.getElementById(labelledBy)?.textContent : el.textContent)?.trim() ?? '';
        return { name, ring: el.matches(':focus-visible') && style.outlineStyle === 'solid' && style.outlineWidth === '2px' };
      });
      if (focused) {
        expect(focused.ring, focused.name).toBe(true);
        reached.push(focused.name);
      }
      await page.keyboard.press('Tab');
    }
    for (const name of ['Close', 'Start at sign-in', 'Match Windows', 'Forget all', 'Forget Edge on Windows', 'Forget Firefox on Windows', 'Reset', 'Check for updates']) {
      expect(reached).toContain(name);
    }
    expect(reached.some((name) => name.startsWith('Troubleshooting'))).toBe(true);
  });

  test('Escape closes Settings from Troubleshooting too', async ({ page }) => {
    await start(page);
    await openTroubleshooting(page);
    await page.keyboard.press('Escape');
    await expect(help(page)).toBeHidden();
    await expect(settings(page)).toBeHidden();
  });
});
