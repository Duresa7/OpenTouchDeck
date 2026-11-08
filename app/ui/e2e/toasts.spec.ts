import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';

const UNREACHABLE = "Can't reach the Companion. Check that Open TouchDeck is running in the tray, then retry.";

/** The demo has no pairing codes, so Open in a browser fails and says so in an error toast. */
async function errorToast(page: Page) {
  await page.addInitScript(() => {
    window.openTouchDeck = { version: 1, session: async () => 'demo', onSessionChanged: () => {} };
  });
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Settings' }).click();
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

test('toasts sit at the bottom centre, clear of the details panel and the nav', async ({ page }) => {
  const error = await errorToast(page);
  await expect(error).toBeVisible();
  const box = (await error.boundingBox())!;
  const width = page.viewportSize()!.width;
  expect(Math.abs(box.x + box.width / 2 - width / 2)).toBeLessThan(2);
});

test('toasts drop their slide when the system asks for reduced motion', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const error = await errorToast(page);
  await expect(error).toBeVisible();
  expect(await error.evaluate((node) => getComputedStyle(node).transitionDuration.split(',').every((d) => parseFloat(d) === 0))).toBe(true);
});
