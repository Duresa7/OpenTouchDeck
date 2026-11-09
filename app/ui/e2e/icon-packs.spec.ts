import { expect, test, type Locator, type Page } from '@playwright/test';
import { created, openCopyTile, openPicker, option, picker } from './icon-picker-helpers.ts';

const nav = (page: Page) => page.getByRole('navigation', { name: 'Main' });
const view = (page: Page) => page.locator('[data-testid="integration-detail"][data-id="icon-packs"]');
const packsRow = (page: Page) => page.locator('[data-testid="integration"][data-id="icon-packs"]');
const row = (page: Page, prefix: string) => view(page).locator(`[data-testid="icon-pack"][data-prefix="${prefix}"]`);
const installed = (page: Page) => page.evaluate(() => window.touchdeckDemo?.iconPacks.installed() ?? []);

const ICONS = JSON.stringify({
  prefix: 'my-icons',
  info: { name: 'My icons', author: { name: 'Me' } },
  icons: { spotify: { body: '<circle cx="12" cy="12" r="10" fill="currentColor"/>' } },
  width: 24,
  height: 24,
});

async function openIntegrations(page: Page, query = '') {
  await page.goto(`/${query}`);
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await nav(page).getByRole('button', { name: 'Integrations' }).click();
  await expect(packsRow(page)).toBeVisible();
}

async function openIconPacks(page: Page, query = '') {
  await openIntegrations(page, query);
  await packsRow(page).getByTestId('integration-settings').click();
  await expect(view(page).getByTestId('icon-packs')).toHaveAttribute('data-state', 'ready');
}

/** Every thumbnail in `scope` drew from its SVG. */
async function drawn(scope: Locator, count: number) {
  const thumbs = scope.getByTestId('pack-thumb');
  await expect(thumbs).toHaveCount(count);
  await expect.poll(() => thumbs.evaluateAll((imgs) => imgs.filter((img) => (img as HTMLImageElement).naturalWidth > 0).length)).toBe(count);
}

test.describe('Integrations, Icon packs', () => {
  test('is an Integration with a switch, off until a pack is installed', async ({ page }) => {
    await openIntegrations(page);
    await expect(packsRow(page)).toContainText('None installed. Icons for the picker, searchable offline.');
    await expect(packsRow(page).getByTestId('integration-switch')).toHaveAttribute('aria-checked', 'false');
    await openIntegrations(page, '?packs=fa6-brands,mdi');
    await expect(packsRow(page)).toContainText('2 installed: ');
    await expect(packsRow(page).getByTestId('integration-switch')).toHaveAttribute('aria-checked', 'true');
  });

  test('turning it on with nothing installed opens pack management, and the first download turns it on', async ({ page }) => {
    await openIntegrations(page);
    await packsRow(page).getByTestId('integration-switch').click();
    await expect(view(page).getByTestId('icon-packs')).toHaveAttribute('data-state', 'ready');
    const header = view(page).getByTestId('detail-switch');
    await expect(header).toHaveAttribute('aria-checked', 'true');
    await expect(header).toHaveAttribute('aria-busy', 'true');
    await row(page, 'lucide').getByTestId('pack-install').click();
    await expect(row(page, 'lucide')).toHaveAttribute('data-installed', 'true');
    await expect(header).not.toHaveAttribute('aria-busy', 'true');
    await view(page).getByTestId('detail-back').click();
    await expect(packsRow(page).getByTestId('integration-switch')).toHaveAttribute('aria-checked', 'true');
    await expect(packsRow(page)).toContainText('1 installed: Lucide');
  });


  test('lists the curated packs with nothing installed, then downloads and removes one', async ({ page }) => {
    await openIconPacks(page);
    await expect(view(page).getByTestId('icon-pack')).toHaveCount(7);
    await expect(view(page).locator('[data-testid="icon-pack"][data-installed="true"]')).toHaveCount(0);
    const brands = row(page, 'fa6-brands');
    await expect(brands).toContainText('Font Awesome 6 Brands');
    await expect(brands).toContainText('495 icons · 498 KB · CC BY 4.0 · Credit required · Logos');
    await expect(brands.getByTestId('pack-license')).toHaveAttribute('href', 'https://creativecommons.org/licenses/by/4.0/');

    await brands.getByRole('button', { name: 'Download Font Awesome 6 Brands' }).click();
    await expect(brands.getByTestId('pack-status')).toBeVisible();
    await expect(brands).toHaveAttribute('data-installed', 'true');
    await expect(brands.getByTestId('pack-status')).toHaveCount(0);
    await drawn(brands, 4);
    expect(await installed(page)).toEqual(['fa6-brands']);

    await brands.getByRole('button', { name: 'Remove Font Awesome 6 Brands' }).click();
    await expect(brands).toHaveAttribute('data-installed', 'false');
    await expect(brands.getByTestId('pack-install')).toHaveText('Download');
    expect(await installed(page)).toEqual([]);
  });

  test('a download that fails its checksum stays offered, with Retry', async ({ page }) => {
    await openIconPacks(page);
    await page.evaluate(() => window.touchdeckDemo?.iconPacks.failInstall('lucide', { code: 'icon_pack.integrity', message: 'The download did not match its pinned checksum.', details: null }));
    const lucide = row(page, 'lucide');
    await lucide.getByTestId('pack-install').click();
    await expect(lucide.getByTestId('pack-problem')).toHaveText("Lucide didn't match its checksum, so it wasn't installed. Retry later.");
    // The Companion's own message waits under Details.
    const details = lucide.getByTestId('pack-problem-details');
    await expect(details.locator('p')).toBeHidden();
    await details.getByText('Details').click();
    await expect(details.locator('p')).toHaveText('The download did not match its pinned checksum.');
    await expect(lucide.getByTestId('pack-install')).toHaveText('Retry');
    await lucide.getByTestId('pack-install').click();
    await expect(lucide).toHaveAttribute('data-installed', 'true');
    await expect(lucide.getByTestId('pack-problem')).toHaveCount(0);
  });

  test('imports a pack file as a custom pack and refuses clashes and other files', async ({ page }) => {
    await openIconPacks(page);
    const file = view(page).getByTestId('pack-import-file');
    await file.setInputFiles({ name: 'icons.json', mimeType: 'application/json', buffer: Buffer.from(ICONS) });
    const mine = row(page, 'my-icons');
    await expect(mine).toContainText('My icons');
    await expect(mine).toContainText('1 icon · ');
    await expect(mine.getByTestId('pack-license')).toHaveText('Custom (you provided it)');
    await drawn(mine, 1);

    await file.setInputFiles({ name: 'again.json', mimeType: 'application/json', buffer: Buffer.from(ICONS) });
    await expect(view(page).getByTestId('pack-import-problem')).toHaveText('A pack with the same prefix as “again.json” is already installed or offered above.');
    await file.setInputFiles({ name: 'logo.svg', mimeType: 'image/svg+xml', buffer: Buffer.from('<svg xmlns="http://www.w3.org/2000/svg"/>') });
    await expect(view(page).getByTestId('pack-import-problem')).toHaveText("“logo.svg” isn't an Iconify JSON pack. Choose an Iconify JSON file, such as icons.json.");
    await expect(view(page).locator('[data-testid="icon-pack"][data-installed="true"]')).toHaveCount(1);
  });

  test('Back returns to Integrations with focus on the Icon packs settings button', async ({ page }) => {
    await openIconPacks(page);
    await view(page).getByTestId('detail-back').click();
    await expect(packsRow(page).getByTestId('integration-settings')).toBeFocused();
  });

  test('Settings no longer lists Icon packs', async ({ page }) => {
    await page.goto('/');
    await nav(page).getByRole('button', { name: 'Settings' }).click();
    const settings = page.getByTestId('settings');
    await expect(settings).toBeVisible();
    await expect(settings).not.toContainText('Icon packs');
  });
});

test.describe('Icon picker with Icon packs', () => {
  test('a search finds the logo in every installed pack, with thumbnails and credit, and a pick becomes the Tile’s Asset', async ({ page }) => {
    await openCopyTile(page, '/?packs=fa6-brands,mdi');
    await openPicker(page);
    await page.getByTestId('icon-search').fill('spotify');
    const fa = option(page, 'hit:iconify:fa6-brands:spotify');
    const mdi = option(page, 'hit:iconify:mdi:spotify');
    await expect(fa).toBeVisible();
    await expect(mdi).toBeVisible();
    await expect(option(page, 'hit:simple-icons:spotify')).toBeVisible();
    await expect(option(page, 'hit:iconify:lucide:spotify')).toHaveCount(0);
    await drawn(fa, 1);
    await drawn(mdi, 1);
    await expect(fa).toHaveAttribute('title', /Font Awesome 6 Brands/);
    await expect(picker(page).getByTestId('icon-credit')).toHaveCount(1);
    await expect(picker(page).getByTestId('icon-credit')).toContainText('Font Awesome 6 Brands by Dave Gandy (CC BY 4.0, modified)');

    await fa.click();
    await expect(picker(page)).toBeHidden();
    expect(await created(page)).toEqual([expect.objectContaining({ kind: 'catalog', catalog: 'iconify:fa6-brands', id: 'spotify' })]);
    await expect(page.getByTestId('tile-icon').locator('canvas[data-ready="true"]')).toBeVisible();
  });

  test('with Icon packs off, the picker hides installed packs and their credit and keeps the bundled icons', async ({ page }) => {
    await openIntegrations(page, '?packs=fa6-brands,mdi');
    await packsRow(page).getByTestId('integration-switch').click();
    await expect(packsRow(page).getByTestId('integration-switch')).toHaveAttribute('aria-checked', 'false');
    await openCopyTile(page, '/?packs=fa6-brands,mdi');
    await openPicker(page);
    await page.getByTestId('icon-search').fill('spotify');
    await expect(option(page, 'hit:simple-icons:spotify')).toBeVisible();
    await expect(option(page, 'hit:iconify:fa6-brands:spotify')).toHaveCount(0);
    await expect(picker(page).getByTestId('icon-credit')).toHaveCount(0);
  });

  test('Get more icons opens Icon packs in Integrations, and a new pack shows up in the next search', async ({ page }) => {
    await openCopyTile(page);
    await openPicker(page);
    await page.getByTestId('icon-search').fill('spotify');
    await expect(option(page, 'hit:simple-icons:spotify')).toBeVisible();
    await expect(option(page, 'hit:iconify:tabler:brand-spotify')).toHaveCount(0);
    await picker(page).getByTestId('icon-more').click();
    await expect(picker(page)).toBeHidden();
    await expect(view(page)).toBeVisible();
    await row(page, 'tabler').getByTestId('pack-install').click();
    await expect(row(page, 'tabler')).toHaveAttribute('data-installed', 'true');
    await nav(page).getByRole('button', { name: 'Layout' }).click();
    await expect(view(page)).toBeHidden();

    await openPicker(page);
    await page.getByTestId('icon-search').fill('spotify');
    await expect(option(page, 'hit:iconify:tabler:brand-spotify')).toBeVisible();
    await drawn(option(page, 'hit:iconify:tabler:brand-spotify'), 1);
  });
});
