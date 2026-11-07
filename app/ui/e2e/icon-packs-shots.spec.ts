import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { openCopyTile, openPicker, option, picker } from './icon-picker-helpers.ts';

const folder = process.env.TD_SHOTS ?? '';

const ICONS = JSON.stringify({ prefix: 'my-icons', info: { name: 'My icons' }, icons: { star: { body: '<path fill="currentColor" d="M12 2l3 7h7l-5.5 4.5L18.5 21 12 16.5 5.5 21l2-7.5L2 9h7z"/>' } }, width: 24, height: 24 });

async function openIconPacks(page: Page, query: string) {
  await page.goto(`/${query}`);
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Settings' }).click();
  await page.getByTestId('settings-icon-packs').click();
  const view = page.getByRole('dialog', { name: 'Icon packs' });
  await expect(view.getByTestId('icon-packs')).toHaveAttribute('data-state', 'ready');
  return view;
}

async function thumbsDrawn(page: Page) {
  await expect.poll(() => page.getByTestId('pack-thumb').evaluateAll((imgs) => imgs.every((img) => (img as HTMLImageElement).naturalWidth > 0) && imgs.length > 0)).toBe(true);
}

for (const theme of ['light', 'dark'] as const) {
  test.describe(theme, () => {
    test.skip(!folder, 'set TD_SHOTS to a folder to take screenshots');
    test.use({ colorScheme: theme, viewport: { width: 1180, height: 900 } });

    test('settings, icon packs entry', async ({ page }) => {
      await page.goto('/');
      await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
      await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Settings' }).click();
      await expect(page.getByTestId('settings-icon-packs')).toBeVisible();
      await page.waitForTimeout(400);
      await page.screenshot({ path: join(folder, `settings-entry-${theme}.png`) });
    });

    test('icon packs, two installed, one downloading, one imported', async ({ page }) => {
      const view = await openIconPacks(page, '?packs=fa6-brands,mdi');
      await view.getByTestId('pack-import-file').setInputFiles({ name: 'icons.json', mimeType: 'application/json', buffer: Buffer.from(ICONS) });
      await expect(view.locator('[data-testid="icon-pack"][data-prefix="my-icons"]')).toBeVisible();
      await thumbsDrawn(page);
      await page.evaluate(() => window.touchdeckDemo?.iconPacks.failInstall('lucide', { code: 'icon_pack.integrity', message: 'The download did not match its pinned checksum.', details: null }));
      await view.locator('[data-testid="icon-pack"][data-prefix="lucide"]').getByTestId('pack-install').click();
      await expect(view.locator('[data-testid="icon-pack"][data-prefix="lucide"]').getByTestId('pack-problem')).toBeVisible();
      await view.locator('[data-testid="icon-pack"][data-prefix="lucide"]').getByTestId('pack-problem-details').getByText('Details').click();
      await view.locator('[data-testid="icon-pack"][data-prefix="logos"]').getByTestId('pack-install').click();
      await expect(view.locator('[data-testid="icon-pack"][data-prefix="logos"]').getByTestId('pack-status')).toBeVisible();
      await page.screenshot({ path: join(folder, `icon-packs-${theme}.png`) });
    });

    test('icon picker, spotify across installed packs', async ({ page }) => {
      await openCopyTile(page, '/?packs=fa6-brands,mdi,tabler,logos');
      await openPicker(page);
      await page.getByTestId('icon-search').fill('spotify');
      await expect(option(page, 'hit:iconify:fa6-brands:spotify')).toBeVisible();
      await expect(option(page, 'hit:iconify:tabler:brand-spotify')).toBeVisible();
      await thumbsDrawn(page);
      await page.waitForTimeout(300);
      await page.screenshot({ path: join(folder, `picker-spotify-${theme}.png`) });
      await expect(picker(page).getByTestId('icon-more')).toBeVisible();
    });
  });
}
