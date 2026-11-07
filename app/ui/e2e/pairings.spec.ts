import { expect, test, type Page } from '@playwright/test';
import { openLayout } from './layout-helpers.ts';

// The demo data: Desk runs the radio build on USB and keeps Pairings with two other apps, Open TouchDeck on Laptop and
// Open TouchDeck on Studio PC. It is on the network at 192.0.2.21, and this app isn't paired with it yet. Studio runs
// the normal build.

const section = (page: Page) => page.getByTestId('device-panel-pairings');
const apps = (page: Page) => page.getByTestId('device-panel-pairing');
const sectionStatus = (page: Page) => page.getByTestId('device-panel-pairings-status');

async function openDesk(page: Page) {
  await openLayout(page);
  await page.getByTestId('device-name').click();
  await expect(page.getByTestId('device-panel')).toBeVisible();
  await expect(apps(page)).toHaveCount(2);
}

async function pairDesk(page: Page) {
  await page.getByTestId('device-panel-wifi-begin').click();
  await expect(page.getByTestId('device-panel-wifi-notice')).toHaveText(
    'Whoever can get to Desk can use every Pairing on it. If Desk is lost or passed on, forget its paired apps.',
  );
  await page.getByTestId('device-panel-wifi-start').click();
  await page.getByTestId('device-panel-wifi-code').fill('123456');
  await page.getByTestId('device-panel-wifi-pair').click();
  await expect(page.getByTestId('device-panel-wifi-forget')).toBeVisible();
}

/** Closes the Device panel and opens it again, which reads the list again. */
async function reopen(page: Page) {
  const panel = page.getByTestId('device-panel');
  await page.getByTestId('device-name').click();
  await expect(panel).toHaveCount(0);
  await page.getByTestId('device-name').click();
  await expect(panel).toBeVisible();
}

const listed = (page: Page) => page.evaluate(() => window.touchdeckDemo?.pairings.list('mock0001').map((p) => p.name));

test('lists the paired apps over USB and forgets one', async ({ page }) => {
  await openDesk(page);
  await expect(page.getByTestId('device-panel-pairings-count')).toHaveText('2 of 4');
  await expect(apps(page).nth(0)).toContainText('Open TouchDeck on Laptop');
  await expect(apps(page).nth(1)).toContainText('Open TouchDeck on Studio PC');
  // Only names show: never a key.
  await expect(section(page)).not.toContainText(/[0-9a-f]{32}/u);

  await apps(page).nth(0).getByTestId('device-panel-pairing-forget').click();
  await expect(apps(page)).toHaveCount(1);
  await expect(page.getByTestId('device-panel-pairings-count')).toHaveText('1 of 4');
  expect(await listed(page)).toEqual(['Open TouchDeck on Studio PC']);
});

test('forgets every app only after the confirm', async ({ page }) => {
  await openDesk(page);
  await page.getByTestId('device-panel-pairings-forget-all').click();
  const confirm = page.getByTestId('device-panel-pairings-confirm');
  await expect(confirm).toContainText('Forget every paired app?');
  await page.getByTestId('device-panel-pairings-confirm-cancel').click();
  await expect(confirm).toHaveCount(0);
  await expect(apps(page)).toHaveCount(2);

  await page.getByTestId('device-panel-pairings-forget-all').click();
  await page.getByTestId('device-panel-pairings-confirm-forget').click();
  await expect(apps(page)).toHaveCount(0);
  await expect(sectionStatus(page)).toHaveText('No apps are paired. Pair one over Wi-Fi to use Desk without the cable.');
  expect(await listed(page)).toEqual([]);
});

test('pairing shows the notice, and the new Pairing is this app in the list', async ({ page }) => {
  await openDesk(page);
  await pairDesk(page);
  // The list reads again once the Pairing is made.
  await expect(apps(page)).toHaveCount(3);
  await expect(apps(page).nth(2)).toHaveAttribute('data-this-app', 'true');
  await expect(apps(page).nth(2)).toContainText('This app');
});

test("forgetting this app on the Device drops this app's Pairing too", async ({ page }) => {
  await openDesk(page);
  await pairDesk(page);
  await expect(apps(page)).toHaveCount(3);
  await apps(page).nth(2).getByTestId('device-panel-pairing-forget').click();
  await expect(apps(page)).toHaveCount(2);
  await expect(page.getByTestId('device-panel-wifi-begin')).toBeVisible();
});

test('a lost answer says so and the next read finishes it', async ({ page }) => {
  await openDesk(page);
  await page.evaluate(() => window.touchdeckDemo?.pairings.loseNextForget('mock0001'));
  await apps(page).nth(1).getByTestId('device-panel-pairing-forget').click();
  // The read right after the lost answer finds the app gone, so no error stays.
  await expect(apps(page)).toHaveCount(1);
  await expect(page.getByTestId('device-panel-pairings-error')).toHaveCount(0);
});

test('a Device that lists four apps over USB gets the advice before pairing', async ({ page }) => {
  await openDesk(page);
  await page.evaluate(() => {
    window.touchdeckDemo?.pairings.add('mock0001', { app: '3c'.repeat(16), name: 'Open TouchDeck on Tablet' });
    window.touchdeckDemo?.pairings.add('mock0001', { app: '4d'.repeat(16), name: 'Open TouchDeck on Office PC' });
  });
  await reopen(page);
  await expect(apps(page)).toHaveCount(4);
  await expect(sectionStatus(page)).toHaveText('Four apps are paired. Forget one to pair another.');
  await page.getByTestId('device-panel-wifi-begin').click();
  const advice = 'Four apps are paired. Forget one first: on Desk, use the paired apps screen, or plug it in with a USB cable.';
  await expect(page.getByTestId('device-panel-wifi-problem')).toHaveText(advice);

  // Forgetting one makes room.
  await page.getByTestId('device-panel-wifi-cancel').click();
  await apps(page).nth(0).getByTestId('device-panel-pairing-forget').click();
  await expect(apps(page)).toHaveCount(3);
  await page.getByTestId('device-panel-wifi-begin').click();
  await expect(page.getByTestId('device-panel-wifi-address-form')).toBeVisible();
  await expect(page.getByTestId('device-panel-wifi-problem')).toHaveCount(0);
});

test('a normal build shows no paired apps', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.getByRole('button', { name: 'Edit Studio' }).click();
  await expect(page.getByTestId('editor')).toHaveAttribute('data-device-id', 'mock0002');
  await page.getByTestId('device-name').click();
  await expect(page.getByTestId('device-panel-update')).toBeVisible();
  await expect(section(page)).toHaveCount(0);
});
