import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { openSettings } from './layout-helpers.ts';

const UNREACHABLE = "Can't reach the Companion. Check that Open TouchDeck is running in the tray, then retry.";

/** The demo has no pairing codes, so Open in a browser fails and says so in an error toast. */
async function errorToast(page: Page) {
  await page.addInitScript(() => {
    window.openTouchDeck = { version: 1, session: async () => 'demo', onSessionChanged: () => {} };
  });
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await openSettings(page, 'browsers');
  await page.getByTestId('open-browser').click();
  return page.locator('[data-sonner-toast]').filter({ hasText: UNREACHABLE });
}

const tokens = (page: Page) =>
  page.evaluate(() => {
    const probe = document.createElement('span');
    document.body.append(probe);
    const read = (name: string, property: 'color' | 'backgroundColor') => {
      probe.style.setProperty(property === 'color' ? 'color' : 'background-color', `var(${name})`);
      return getComputedStyle(probe)[property];
    };
    const out = { surface: read('--surface-raised', 'backgroundColor'), ink: read('--ink', 'color'), danger: read('--danger-ink', 'color') };
    probe.remove();
    return out;
  });

for (const theme of ['light', 'dark'] as const) {
  test(`toasts use the app's surface, type and status colors in ${theme}`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: theme });
    const error = await errorToast(page);
    await expect(error).toBeVisible();
    const want = await tokens(page);
    const look = (toast: typeof error) =>
      toast.evaluate((node) => {
        const style = getComputedStyle(node);
        const icon = node.querySelector('[data-icon]');
        return {
          background: style.backgroundColor,
          color: style.color,
          radius: style.borderRadius,
          font: style.fontFamily,
          size: style.fontSize,
          styled: node.getAttribute('data-styled'),
          icon: icon ? getComputedStyle(icon).color : null,
        };
      });
    const errorLook = await look(error);
    expect(errorLook).toMatchObject({ background: want.surface, color: want.ink, radius: '12px', size: '14px', styled: 'false', icon: want.danger });
    expect(errorLook.font).toContain('Archivo');
  });

  test(`proof toasts ${theme}`, async ({ page }) => {
    test.skip(!process.env.TD_SHOTS, 'set TD_SHOTS for visual evidence');
    await page.emulateMedia({ colorScheme: theme });
    const error = await errorToast(page);
    await expect(error).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    await page.waitForTimeout(600);
    await page.screenshot({ path: join(process.env.TD_SHOTS!, `toast-error-${theme}.png`) });
  });
}

test('toasts sit at the bottom centre of the content right of the rail, with no room kept for the old dock', async ({ page }) => {
  for (const size of [{ width: 800, height: 600 }, { width: 1280, height: 800 }]) {
    await page.setViewportSize(size);
    const error = await errorToast(page);
    await expect(error).toBeVisible();
    const rail = (await page.locator('.nav-rail').boundingBox())!;
    const content = rail.x + rail.width + (size.width - rail.x - rail.width) / 2;
    await expect
      .poll(async () => {
        const settled = (await error.boundingBox())!;
        return { centre: Math.abs(settled.x + settled.width / 2 - content) < 2, clear: settled.x > rail.x + rail.width, gap: Math.round(size.height - settled.y - settled.height) };
      }, { message: `the toast at ${size.width} × ${size.height}` })
      .toEqual({ centre: true, clear: true, gap: 24 });
  }
});

test('toasts drop their slide when the system asks for reduced motion', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const error = await errorToast(page);
  await expect(error).toBeVisible();
  expect(await error.evaluate((node) => getComputedStyle(node).transitionDuration.split(',').every((d) => parseFloat(d) === 0))).toBe(true);
});
