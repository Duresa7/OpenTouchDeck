import { expect, test, type Page } from '@playwright/test';
import { openSettings, settingsPage, type SettingsCategory } from './layout-helpers.ts';

const nav = (page: Page) => page.getByRole('navigation', { name: 'Main' });
const navButton = (page: Page, name: string) => nav(page).getByRole('button', { name, exact: true });
const settings = settingsPage;
const title = (page: Page) => page.getByTestId('settings-title');
const confirmation = (page: Page) => page.getByRole('alertdialog');
const startSwitch = (page: Page) => settings(page).getByRole('switch', { name: 'Start at sign-in' });
const browsers = (page: Page) => settings(page).getByTestId('paired-browser');
const topics = (page: Page) => page.evaluate(() => window.touchdeckDemo?.topics() ?? []);

async function start(page: Page, query = '') {
  await page.goto(`/${query}`);
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
}

async function general(page: Page) {
  await openSettings(page);
  await expect(startSwitch(page)).toBeEnabled();
}

/** Leaves Settings for Home, so the next visit starts the page afresh. */
async function leave(page: Page) {
  await navButton(page, 'Home').click();
  await expect(settings(page)).toHaveCount(0);
}

test.describe('the Settings page', () => {
  test('the nav opens it at General, with a category for each Device, and marks where you are', async ({ page }) => {
    await start(page);
    await general(page);
    await expect(navButton(page, 'Settings')).toHaveAttribute('aria-current', 'page');
    await expect(navButton(page, 'Home')).not.toHaveAttribute('aria-current');
    await expect(page.getByRole('list', { name: 'Devices' }), 'Settings takes the whole window').toHaveCount(0);
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect(title(page)).toHaveText('General');
    await expect(page.getByTestId('settings-nav-general')).toHaveAttribute('aria-current', 'page');
    await expect(settings(page)).toContainText('Appearance');
    await expect(settings(page).getByRole('button', { name: 'Close' })).toHaveCount(0);
    await expect(settings(page).getByRole('button', { name: 'Back' })).toHaveCount(0);

    const devices = page.getByTestId('settings-nav-device');
    await expect(devices).toHaveText(['Desk', 'Studio', 'Stream deck by the window']);
    expect(await devices.evaluateAll((els) => els.map((el) => el.getAttribute('data-device')))).toEqual(['mock0001', 'mock0002', 'mock0003']);

    const categories: [SettingsCategory, string][] = [
      ['browsers', 'Paired browsers'],
      ['troubleshooting', 'Troubleshooting'],
      ['errors', 'Errors'],
      ['about', 'About'],
      ['general', 'General'],
    ];
    for (const [category, words] of categories) {
      await page.getByTestId(`settings-nav-${category}`).click();
      await expect(settings(page)).toHaveAttribute('data-section', category);
      await expect(title(page)).toHaveText(words);
      await expect(title(page), 'focus moves to the new title').toBeFocused();
      await expect(page.locator('.settings-nav-item[aria-current="page"]')).toHaveCount(1);
      await expect(page.getByTestId(`settings-nav-${category}`)).toHaveAttribute('aria-current', 'page');
    }
    await expect(page.getByTestId('errors-empty')).toHaveCount(0);
    await page.getByTestId('settings-nav-errors').click();
    await expect(page.getByTestId('errors-empty')).toHaveText('No errors.');

    await devices.nth(1).click();
    await expect(settings(page)).toHaveAttribute('data-section', 'device');
    await expect(title(page)).toHaveText('Studio');
    await expect(devices.nth(1)).toHaveAttribute('aria-current', 'page');
    await expect(devices.nth(0)).not.toHaveAttribute('aria-current');
    await expect(page.getByTestId('device-panel')).toHaveAttribute('data-device', 'mock0002');

    await page.getByTestId('settings-nav-about').click();
    await leave(page);
    await navButton(page, 'Settings').click();
    await expect(settings(page), 'the nav always opens General').toHaveAttribute('data-section', 'general');
  });

  test('with no Device, the Devices category says how to connect one', async ({ page }) => {
    await page.goto('/?devices=0');
    await openSettings(page);
    const device = page.getByTestId('settings-nav-device');
    await expect(device).toHaveCount(1);
    await expect(device).toHaveText('No Device');
    await device.click();
    await expect(settings(page)).toHaveAttribute('data-section', 'device');
    await expect(title(page)).toHaveText('Devices');
    await expect(settings(page)).toContainText('No Device connected. Plug one in with a USB cable.');
    await expect(page.getByTestId('device-panel')).toHaveCount(0);
  });

  test('Escape leaves Settings where it is', async ({ page }) => {
    await start(page);
    await openSettings(page, 'troubleshooting');
    await page.keyboard.press('Escape');
    await expect(settings(page)).toBeVisible();
    await expect(settings(page)).toHaveAttribute('data-section', 'troubleshooting');
    await expect(navButton(page, 'Settings')).toHaveAttribute('aria-current', 'page');
  });
});

test.describe('Start at sign-in', () => {
  test('toggles and is still set when Settings opens again', async ({ page }) => {
    await start(page);
    await general(page);
    await expect(startSwitch(page)).not.toBeChecked();
    await expect(settings(page)).toContainText('Runs in the tray when you sign in to Windows, so your Devices work right away.');

    await startSwitch(page).click();
    await expect(startSwitch(page)).toBeChecked();
    await leave(page);

    await general(page);
    await expect(startSwitch(page)).toBeChecked();
    await startSwitch(page).click();
    await leave(page);
    await general(page);
    await expect(startSwitch(page)).not.toBeChecked();
  });
});

test.describe('Paired browsers', () => {
  test('lists each browser with its last use, and Forget asks first, then removes it for good', async ({ page }) => {
    await start(page);
    await openSettings(page, 'browsers');
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

    await leave(page);
    await openSettings(page, 'browsers');
    await expect(browsers(page)).toHaveCount(1);
    await expect(browsers(page)).toContainText('Firefox on Windows');
  });

  test('Forget all removes every browser and shows how to pair one', async ({ page }) => {
    await start(page);
    await openSettings(page, 'browsers');
    await settings(page).getByRole('button', { name: 'Forget all' }).click();
    await expect(confirmation(page)).toHaveAccessibleName('Forget all browsers?');
    await confirmation(page).getByRole('button', { name: 'Forget all' }).click();
    await expect(browsers(page)).toHaveCount(0);
    await expect(settings(page)).toContainText('No browsers paired yet.');
  });

  test('Reset pairing confirms first, then signs out every browser while this window carries on', async ({ page }) => {
    await start(page);
    await openSettings(page, 'browsers');
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

    await leave(page);
    await expect(page.getByRole('listitem')).toHaveCount(3);
    await openSettings(page, 'about');
    await expect(settings(page)).toContainText('Open TouchDeck 0.2.0');
  });

  test('the desktop window offers to open a browser here, and a browser does not', async ({ page }) => {
    await start(page);
    await openSettings(page, 'browsers');
    await expect(settings(page).getByTestId('open-browser-row')).toHaveCount(0);
    await page.addInitScript(() => {
      window.openTouchDeck = { version: 1, session: async () => 'demo', onSessionChanged: () => {} };
    });
    await start(page);
    await expect(page.locator('.open-browser')).toHaveCount(0);
    await openSettings(page, 'browsers');
    const row = settings(page).getByTestId('open-browser-row');
    await expect(row).toContainText('Open in a browser');
    await expect(row).toContainText('Pairs a browser on this PC.');
    await expect(row.getByRole('button', { name: 'Open' })).toBeVisible();
  });

  test('Icon packs and Brandfetch logos moved to Integrations', async ({ page }) => {
    await start(page);
    for (const category of ['general', 'browsers', 'troubleshooting', 'about'] as const) {
      await openSettings(page, category);
      await expect(settings(page)).not.toContainText('Icon packs');
      await expect(settings(page)).not.toContainText('Brandfetch');
    }
  });

  test('with none paired it says how to pair one', async ({ page }) => {
    await start(page, '?browsers=0');
    await openSettings(page, 'browsers');
    await expect(settings(page)).toContainText('No browsers paired yet.');
    await expect(settings(page).getByRole('button', { name: 'Forget all' })).toHaveCount(0);
  });
});

test.describe('Troubleshooting', () => {
  test('shows each Device with its link and Firmware fields', async ({ page }) => {
    await start(page);
    await openSettings(page, 'troubleshooting');
    await expect(settings(page)).toContainText('Device details, and tools to check that taps reach this PC.');
    const details = settings(page).getByTestId('device-details');
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
  });

  test("Connection help on an empty Home opens Troubleshooting, and the nav's Settings opens General", async ({ page }) => {
    await page.goto('/?devices=0');
    await page.getByRole('button', { name: 'Connection help' }).click();
    await expect(settings(page)).toHaveAttribute('data-section', 'troubleshooting');
    await expect(page.getByTestId('settings-nav-troubleshooting')).toHaveAttribute('aria-current', 'page');
    await expect(settings(page)).toContainText('No Device connected. Plug one in with a USB cable.');
    await expect(settings(page).getByRole('heading', { name: 'Firmware console' })).toHaveCount(0);
    await navButton(page, 'Settings').click();
    await expect(settings(page)).toHaveAttribute('data-section', 'general');
  });

  test('a console line round-trips, and the console listens only while it is open', async ({ page }) => {
    await start(page);
    expect(await topics(page)).toEqual([]);
    await openSettings(page, 'troubleshooting');
    await expect.poll(() => topics(page)).toEqual(['console/mock0001']);
    await expect(settings(page)).toContainText("Send a command to Desk's console, like version or layout state.");

    const field = settings(page).getByRole('textbox', { name: 'Console command' });
    const log = settings(page).getByTestId('console-log');
    await field.fill('version');
    await settings(page).getByRole('button', { name: 'Send' }).click();
    await expect(log.locator('[data-kind="sent"]')).toHaveText(['> version']);
    await expect(log.locator('[data-kind="line"]')).toHaveCount(1);
    await expect(log.locator('[data-kind="line"]')).toContainText('"cmd":"version"');
    await expect(field).toHaveValue('');

    await settings(page).getByRole('group', { name: 'Device' }).getByRole('button', { name: 'Studio' }).click();
    await expect.poll(() => topics(page)).toEqual(['console/mock0002']);
    await expect(log).toContainText('Nothing yet. Send a command below.');

    await page.getByTestId('settings-nav-general').click();
    await expect.poll(() => topics(page)).toEqual([]);
  });

  test('a line off the allowlist is not sent, and the console lists what it takes', async ({ page }) => {
    await start(page);
    await openSettings(page, 'troubleshooting');
    const field = settings(page).getByRole('textbox', { name: 'Console command' });
    await field.fill('reboot');
    await field.press('Enter');
    const notice = settings(page).getByTestId('console-forbidden');
    await expect(notice).toContainText('“reboot” wasn\'t sent. The console takes only these read-only commands:');
    await expect(notice.getByRole('listitem')).toHaveCount(23);
    await expect(notice).toContainText('layout state');
    await expect(settings(page).getByTestId('console-log').locator('[data-kind]')).toHaveCount(0);
    await expect(field).toHaveValue('reboot');

    await field.fill('heap');
    await field.press('Enter');
    await expect(notice).toBeHidden();
    await expect(settings(page).getByTestId('console-log').locator('[data-kind="line"]')).toContainText('free_heap');
  });

  test('Update from a file updates a Device from the chosen file, and a file that is not Firmware says so', async ({ page }) => {
    await start(page);
    await openSettings(page, 'troubleshooting');
    await expect(settings(page).getByRole('heading', { name: 'Update from a file' })).toBeVisible();
    const rows = settings(page).getByTestId('firmware-file');
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
    await expect(settings(page).getByTestId('device-details').nth(1).locator('[data-fact="version"] dd')).toHaveText('1.5.0');

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
    await openSettings(page, 'about');
    await expect(settings(page)).toContainText('Open TouchDeck 0.2.0');
    await expect(settings(page)).toContainText('Build demo');
    await settings(page).getByRole('button', { name: 'Check for updates' }).click();
    await expect(settings(page)).toContainText('Checking…');
    await expect(settings(page)).toContainText('Up to date');
  });
});

test.describe('keyboard', () => {
  /** Tabs from a sidebar entry, and lists each control reached inside Settings, checking its focus ring. */
  async function tabFrom(page: Page, testId: string, presses: number) {
    await page.getByTestId(testId).focus();
    const reached: string[] = [];
    for (let i = 0; i < presses; i++) {
      const focused = await page.evaluate(() => {
        const el = document.activeElement;
        if (!(el instanceof HTMLElement) || !el.closest('[data-testid="settings"]')) return null;
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
    return reached;
  }

  test('Tab reaches every category and every Settings control with the visible focus ring', async ({ page }) => {
    await start(page);
    await navButton(page, 'Settings').focus();
    await page.keyboard.press('Enter');
    await expect(settings(page)).toHaveAttribute('data-section', 'general');
    await expect(title(page), 'focus lands on the page title').toBeFocused();
    await expect(startSwitch(page)).toBeEnabled();
    expect(await tabFrom(page, 'settings-nav-general', 12)).toEqual(
      expect.arrayContaining(['General', 'Desk', 'Studio', 'Stream deck by the window', 'Paired browsers', 'Troubleshooting', 'Errors', 'About', 'Start at sign-in', 'Match Windows']),
    );

    await page.getByTestId('settings-nav-browsers').focus();
    await page.keyboard.press('Enter');
    await expect(browsers(page)).toHaveCount(2);
    expect(await tabFrom(page, 'settings-nav-browsers', 10)).toEqual(
      expect.arrayContaining(['Paired browsers', 'Troubleshooting', 'Errors', 'About', 'Forget all', 'Forget Edge on Windows', 'Forget Firefox on Windows', 'Reset']),
    );

    await page.getByTestId('settings-nav-about').focus();
    await page.keyboard.press('Enter');
    expect(await tabFrom(page, 'settings-nav-about', 3)).toEqual(['About', 'Check for updates']);
  });
});
