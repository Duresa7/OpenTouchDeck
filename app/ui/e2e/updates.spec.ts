import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import type { FakeUpdatesScript } from '../src/lib/companion/fake.ts';
import { openSettings, settingsPage } from './layout-helpers.ts';

const demo = (page: Page) => ({
  script: (next: Partial<FakeUpdatesScript>) => page.evaluate((next) => window.touchdeckDemo?.updates.script(next), next),
  finish: () => page.evaluate(() => window.touchdeckDemo?.updates.finish()),
  calls: () => page.evaluate(() => ({ check: 0, download: 0, install: 0, ...window.touchdeckDemo?.updates.calls })),
});

const offer = { state: 'available', version: '1.1.0', current: '0.2.0', notes: '- One change', date: null } as const;

// The modal dialog hides the nav from the accessibility tree, so the icon is found by its class.
const icon = (page: Page) => page.locator('.update-button');
const AT_VISIBLE_ICON = { position: { x: 22, y: 22 } };
const slot = (page: Page) => page.locator('.update-slot');
const label = (page: Page) => page.locator('.update-label');
const dialog = (page: Page) => page.getByRole('dialog');
const settings = settingsPage;
const nav = (page: Page) => page.getByRole('navigation', { name: 'Main' });

async function start(page: Page, update = 'available') {
  await page.goto(`/?update=${update}`);
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
}

async function openOffer(page: Page) {
  await icon(page).click(AT_VISIBLE_ICON);
  await expect(dialog(page)).toHaveAccessibleName('Open TouchDeck 1.1.0');
}

test.describe('the Update icon', () => {
  test('is not rendered when no update exists, and About says up to date', async ({ page }) => {
    await start(page, 'none');
    await expect.poll(async () => (await demo(page).calls()).check).toBe(1);
    await expect(icon(page)).toHaveCount(0);
    await openSettings(page, 'about');
    await expect(settings(page)).toContainText('Up to date');
  });

  test('shows a dot when an update exists, opens on mouse hover without moving the nav, and closes on leave', async ({ page }) => {
    await start(page);
    const button = nav(page).getByRole('button', { name: 'Update available' });
    await expect(button).toBeVisible();
    await expect(page.locator('.update-dot')).toBeVisible();
    await expect(label(page)).toHaveCSS('opacity', '0');

    const puck = (await page.locator('.nav-puck').boundingBox())!;
    const slotBox = (await slot(page).boundingBox())!;
    expect(Math.round(slotBox.x - (puck.x + puck.width))).toBe(8);
    const home = await nav(page).getByRole('button', { name: 'Home' }).boundingBox();

    await button.hover(AT_VISIBLE_ICON);
    await expect(slot(page)).toHaveClass(/is-expanded/);
    await expect(label(page)).toHaveCSS('opacity', '1');
    await expect(label(page)).toHaveText('Update available');
    expect(await nav(page).getByRole('button', { name: 'Home' }).boundingBox()).toEqual(home);
    await expect(page.locator('.tooltip')).toHaveCount(0);

    await page.mouse.move(600, 300);
    await expect(slot(page)).not.toHaveClass(/is-expanded/);
    await expect(label(page)).toHaveCSS('opacity', '0');
  });

  test('stops expanding after Later, keeps its dot, and a tooltip names it', async ({ page }) => {
    await start(page);
    await openOffer(page);
    await dialog(page).getByRole('button', { name: 'Later' }).click();
    await expect(dialog(page)).toBeHidden();
    await expect(page.locator('.update-dot')).toBeVisible();

    await icon(page).hover(AT_VISIBLE_ICON);
    await expect(page.locator('.tooltip')).toHaveText('Update available');
    await expect(slot(page)).not.toHaveClass(/is-expanded/);
  });

  test('turns into a progress ring while downloading, then Ready to restart', async ({ page }) => {
    await start(page);
    await demo(page).script({ holdAt: 0.62 });
    await openOffer(page);
    await dialog(page).getByRole('button', { name: 'Update' }).click();
    await expect(dialog(page).getByRole('progressbar')).toHaveAttribute('aria-valuenow', '62');
    await dialog(page).getByRole('button', { name: 'Hide' }).click();
    await expect(dialog(page)).toBeHidden();

    const downloading = nav(page).getByRole('button', { name: 'Downloading… 62%' });
    await expect(downloading).toBeVisible();
    await expect(page.locator('.update-ring-fill')).toBeVisible();
    await expect(page.locator('.update-dot')).toHaveCount(0);
    await downloading.hover(AT_VISIBLE_ICON);
    await expect(label(page)).toHaveText('Downloading… 62%');

    await demo(page).finish();
    await expect(nav(page).getByRole('button', { name: 'Ready to restart' })).toBeVisible();
    await expect(page.locator('.update-dot')).toBeVisible();
  });
});

test.describe('the Update dialog', () => {
  test('offers the version with its changes as plain text, and nothing downloads before Update', async ({ page }) => {
    await start(page);
    await openOffer(page);
    await expect(dialog(page)).toContainText('You have 0.2.0.');
    await expect(dialog(page).getByRole('heading', { name: "What's new" })).toBeVisible();
    await expect(dialog(page).getByRole('listitem')).toHaveText([
      'Volume Tiles follow your finger as you drag.',
      'Devices reconnect faster after this PC wakes.',
    ]);
    await expect(dialog(page)).not.toContainText('##');
    expect((await demo(page).calls()).download).toBe(0);
  });

  test('says when no release notes were published', async ({ page }) => {
    await start(page, 'no-notes');
    await openOffer(page);
    await expect(dialog(page)).toContainText('No release notes.');
    await expect(dialog(page).getByRole('listitem')).toHaveCount(0);
  });

  test('shows markup in release notes as text', async ({ page }) => {
    await start(page);
    await demo(page).script({ check: { ...offer, notes: '- <img src=x onerror="document.title=1"> <b>bold</b>' } });
    await openSettings(page, 'about');
    await settings(page).getByRole('button', { name: 'Check for updates' }).click();
    await expect.poll(async () => (await demo(page).calls()).check).toBe(2);
    await icon(page).click(AT_VISIBLE_ICON);
    await expect(dialog(page).getByRole('listitem')).toHaveText('<img src=x onerror="document.title=1"> <b>bold</b>');
    await expect(dialog(page).locator('img, b')).toHaveCount(0);
    await expect(page).toHaveTitle('Open TouchDeck');
  });

  test('downloads with progress in percent and megabytes, then Restart restarts to install', async ({ page }) => {
    await start(page);
    await demo(page).script({ holdAt: 0.62 });
    await openOffer(page);
    await dialog(page).getByRole('button', { name: 'Update' }).click();
    await expect(dialog(page)).toContainText('You can keep working.');
    await expect(page.locator('.update-progress-meta')).toHaveText('38 MB of 61 MB62%');
    await expect(dialog(page).getByRole('progressbar')).toHaveAttribute('aria-valuetext', 'Downloading… 62%');

    await demo(page).finish();
    await expect(dialog(page)).toHaveAccessibleName('Ready to restart');
    await expect(dialog(page)).toContainText('Open TouchDeck restarts to install. Your Layouts and settings stay.');
    await dialog(page).getByRole('button', { name: 'Restart' }).click();
    await expect(dialog(page)).toContainText('Restarting to install…');
    await expect(dialog(page).getByRole('button', { name: 'Restart' })).toHaveAttribute('aria-disabled', 'true');
    expect(await demo(page).calls()).toEqual({ check: 1, download: 1, install: 1 });
  });

  test('shows kilobytes when the download size is unknown', async ({ page }) => {
    await start(page);
    await demo(page).script({ size: null, holdAt: 0.5 });
    await openOffer(page);
    await dialog(page).getByRole('button', { name: 'Update' }).click();
    await expect(page.locator('.update-progress-meta')).toHaveText('Downloading… 31250 KB');
    await expect(dialog(page).getByRole('progressbar')).not.toHaveAttribute('aria-valuenow');
  });

  const failures = [
    [{ state: 'failed', reason: 'signature' }, "The download didn't match the project's signing key, so it wasn't installed. Retry later."],
    [{ state: 'failed', reason: 'download' }, "Couldn't download the update. Check your internet connection, then retry."],
    [{ state: 'offline' }, "You're offline. Connect to the internet, then retry."],
  ] as const;
  for (const [outcome, message] of failures) {
    test(`a download that ends ${outcome.state} ${'reason' in outcome ? outcome.reason : ''} says so, and Retry tries again`, async ({ page }) => {
      await start(page);
      await demo(page).script({ download: outcome, stepMs: 5 });
      await openOffer(page);
      await dialog(page).getByRole('button', { name: 'Update' }).click();
      await expect(dialog(page).getByRole('alert')).toHaveText(message);
      await expect(icon(page)).toHaveAttribute('aria-label', 'Update available');

      await demo(page).script({ download: { state: 'ready' } });
      await dialog(page).getByRole('button', { name: 'Retry' }).click();
      await expect(dialog(page)).toHaveAccessibleName('Ready to restart');
      expect((await demo(page).calls()).download).toBe(2);
    });
  }

  test('Restart waits for a busy Device by name, and an installer that will not start can be retried', async ({ page }) => {
    await start(page);
    await demo(page).script({ install: { state: 'failed', reason: 'busy', device: 'mock0001' }, stepMs: 5 });
    await openOffer(page);
    await dialog(page).getByRole('button', { name: 'Update' }).click();
    await dialog(page).getByRole('button', { name: 'Restart' }).click();
    await expect(dialog(page)).toContainText('Wait for Desk to finish sending.');

    await demo(page).script({ install: { state: 'failed', reason: 'installer' } });
    await dialog(page).getByRole('button', { name: 'Restart' }).click();
    await expect(dialog(page).getByRole('alert')).toHaveText("Couldn't start the installer. Retry, or download it from the releases page.");

    await demo(page).script({ install: 'quit' });
    await dialog(page).getByRole('button', { name: 'Retry' }).click();
    await expect(dialog(page)).toContainText('Restarting to install…');
    expect(await demo(page).calls()).toEqual({ check: 1, download: 1, install: 3 });
  });
});

test.describe('checking from Settings, About', () => {
  test('an offline check shows no icon, says offline, and Check for updates finds the update', async ({ page }) => {
    await start(page, 'offline');
    await openSettings(page, 'about');
    await expect(settings(page)).toContainText("You're offline. Connect to the internet, then retry.");
    await expect(icon(page)).toHaveCount(0);

    await demo(page).script({ check: offer, stepMs: 400 });
    await settings(page).getByRole('button', { name: 'Check for updates' }).click();
    await expect(settings(page)).toContainText('Checking…');
    await expect(settings(page).getByRole('button', { name: 'Check for updates' })).toHaveAttribute('aria-disabled', 'true');
    await expect(icon(page)).toHaveCount(1);
    await expect(settings(page)).not.toContainText('Checking…');
    await expect(nav(page).getByRole('button', { name: 'Update available' })).toBeVisible();
  });

  test('a failed check says the update server is unreachable', async ({ page }) => {
    await start(page, 'failed');
    await openSettings(page, 'about');
    await expect(settings(page)).toContainText("Can't reach the update server. Check your internet connection, then retry.");
  });

  test('a source build without a public key offers no check', async ({ page }) => {
    await start(page, 'disabled');
    await expect.poll(async () => (await demo(page).calls()).check).toBe(1);
    await openSettings(page, 'about');
    await expect(settings(page).getByRole('button', { name: 'Check for updates' })).toHaveCount(0);
    await expect(icon(page)).toHaveCount(0);
  });
});

test.describe('keyboard and focus', () => {
  test('Tab opens the icon at once with a visible ring, Enter lands on the first control, Escape returns focus', async ({ page }) => {
    await start(page);
    await nav(page).getByRole('button', { name: 'Settings' }).focus();
    await page.keyboard.press('Tab');
    const button = nav(page).getByRole('button', { name: 'Update available' });
    await expect(button).toBeFocused();
    await expect(slot(page)).toHaveClass(/is-expanded/);
    await expect(slot(page)).toHaveClass(/is-instant/);
    const focus = await button.evaluate((el) => {
      const style = getComputedStyle(el);
      return { visible: el.matches(':focus-visible'), outline: `${style.outlineStyle} ${style.outlineWidth}`, clip: style.clipPath, duration: style.transitionDuration };
    });
    expect(focus).toMatchObject({ visible: true, outline: 'solid 2px', duration: '0s' });
    expect(focus.clip).toMatch(/^inset\(-4px/);

    await page.keyboard.press('Enter');
    await expect(dialog(page)).toBeVisible();
    const later = dialog(page).getByRole('button', { name: 'Later' });
    await expect(later).toBeFocused();
    expect(await later.evaluate((el) => el.matches(':focus-visible'))).toBe(true);

    await page.keyboard.press('Escape');
    await expect(dialog(page)).toBeHidden();
    await expect(button).toBeFocused();
  });

  test('opened with the pointer, the dialog itself holds focus', async ({ page }) => {
    await start(page);
    await openOffer(page);
    await expect(dialog(page)).toBeFocused();
    await expect(dialog(page).locator(':focus-visible')).toHaveCount(0);
  });

  test('focus stays in the dialog when Update replaces its buttons', async ({ page }) => {
    await start(page);
    await demo(page).script({ holdAt: 0.3 });
    await icon(page).focus();
    await page.keyboard.press('Enter');
    await page.keyboard.press('Tab');
    await expect(dialog(page).getByRole('button', { name: 'Update' })).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(dialog(page).getByRole('progressbar')).toBeVisible();
    expect(await dialog(page).evaluate((el) => el.contains(document.activeElement))).toBe(true);
    await page.keyboard.press('Tab');
    await expect(dialog(page).getByRole('button', { name: 'Hide' })).toBeFocused();
  });

  test('with reduced motion the icon jumps open on hover', async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await start(page);
    await icon(page).hover(AT_VISIBLE_ICON);
    await expect(slot(page)).toHaveClass(/is-expanded/);
    expect(await icon(page).evaluate((el) => getComputedStyle(el).transitionDuration)).toBe('0s');
    expect(await label(page).evaluate((el) => getComputedStyle(el).opacity)).toBe('1');
  });
});

test('the update flow runs without console errors or warnings', async ({ page }) => {
  const problems: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error' || message.type() === 'warning') problems.push(message.text());
  });
  page.on('pageerror', (error) => problems.push(error.message));
  await start(page);
  await demo(page).script({ stepMs: 5 });
  await openOffer(page);
  await dialog(page).getByRole('button', { name: 'Update' }).click();
  await expect(dialog(page)).toHaveAccessibleName('Ready to restart');
  await dialog(page).getByRole('button', { name: 'Later' }).click();
  await expect(dialog(page)).toBeHidden();
  expect(problems).toEqual([]);
});

const folder = process.env.TD_SHOTS;

for (const theme of ['light', 'dark'] as const) {
  test.describe(`screenshots, ${theme}`, () => {
    test.skip(!folder, 'set TD_SHOTS to a folder to take screenshots');
    test.use({ colorScheme: theme });

    const shoot = async (page: Page, name: string, { hover = false } = {}) => {
      if (!hover) await page.mouse.move(600, 120);
      await page.evaluate(() => document.fonts.ready);
      await page.waitForTimeout(400);
      await page.screenshot({ path: join(folder!, `${name}-${theme}.png`) });
    };

    test.beforeEach(async ({ page }) => {
      await start(page);
      await expect(icon(page)).toBeVisible();
    });

    test('the icon', async ({ page }) => {
      await shoot(page, 'update-available');
      await icon(page).hover(AT_VISIBLE_ICON);
      await shoot(page, 'update-hovered', { hover: true });
    });

    test('keyboard focus on the icon', async ({ page }) => {
      await nav(page).getByRole('button', { name: 'Settings' }).focus();
      await page.keyboard.press('Tab');
      await shoot(page, 'update-focus');
    });

    test('the dialog at each step', async ({ page }) => {
      await demo(page).script({ holdAt: 0.62 });
      await openOffer(page);
      await shoot(page, 'update-dialog');
      await dialog(page).getByRole('button', { name: 'Update' }).click();
      await expect(dialog(page).getByRole('progressbar')).toHaveAttribute('aria-valuenow', '62');
      await shoot(page, 'update-downloading');
      await dialog(page).getByRole('button', { name: 'Hide' }).click();
      await icon(page).hover(AT_VISIBLE_ICON);
      await shoot(page, 'update-downloadingdock', { hover: true });
      await page.mouse.move(600, 120);
      await demo(page).finish();
      await icon(page).click(AT_VISIBLE_ICON);
      await expect(dialog(page)).toHaveAccessibleName('Ready to restart');
      await shoot(page, 'update-ready');
    });

    test('a download refused by the signature check', async ({ page }) => {
      await demo(page).script({ download: { state: 'failed', reason: 'signature' }, stepMs: 5 });
      await openOffer(page);
      await dialog(page).getByRole('button', { name: 'Update' }).click();
      await expect(dialog(page).getByRole('alert')).toBeVisible();
      await shoot(page, 'update-signature');
    });
  });
}
