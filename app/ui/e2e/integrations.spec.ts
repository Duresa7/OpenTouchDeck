import { expect, test, type Page } from '@playwright/test';

const KEY = 'e2e-secret-5c1d70';

type FakeRecord = { settings: { enabled: boolean; url: string; interval_s: number }; keySet: boolean; refreshes: number; configPuts: number; enabledPuts: number };

async function record(page: Page): Promise<FakeRecord> {
  return page.evaluate(async () => (await fetch('/api/v1/test/integrations/record')).json()) as Promise<FakeRecord>;
}

async function openIntegrations(page: Page, query = '') {
  await page.goto(`/${query}`);
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.getByRole('button', { name: 'Integrations', exact: true }).click();
  await expect(page.getByTestId('integration-list')).toBeVisible();
}

const row = (page: Page, id: string) => page.locator(`[data-testid="integration"][data-id="${id}"]`);

async function openCliProxy(page: Page, query = '') {
  await openIntegrations(page, query);
  await row(page, 'cliproxyapi').getByTestId('integration-settings').click();
  await expect(page.getByTestId('integration-detail')).toHaveAttribute('data-id', 'cliproxyapi');
}

const field = (page: Page, name: string) => page.locator(`[data-testid="config-field"][data-field="${name}"]`);
const outcome = (page: Page, at: 'settings' | 'secret' | 'status' | 'page') => page.locator(`[data-testid="outcome"][data-at="${at}"]`);

async function reach(page: Page, testId: string, nth = 0) {
  const el = page.getByTestId(testId).nth(nth);
  await el.focus();
  await page.keyboard.press('Tab');
  await page.keyboard.press('Shift+Tab');
  await expect(el).toBeFocused();
  return el.evaluate((node) => {
    const style = getComputedStyle(node);
    return { visible: node.matches(':focus-visible'), ring: style.outlineStyle !== 'none' || style.boxShadow.includes('inset') };
  });
}

async function startOnShortcuts(page: Page, query = '') {
  await page.goto(`/${query}`);
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.getByRole('button', { name: 'Layout', exact: true }).click();
  await page.getByTestId('start-starter').click();
  await page.getByTestId('page-tab').filter({ hasText: 'Shortcuts' }).click();
  await page.locator('[data-testid="empty-cell"][data-row="2"][data-col="2"]').click();
}

test('lists each Integration with an on/off switch, and a gear only where there are settings', async ({ page }) => {
  await openIntegrations(page);
  const cli = row(page, 'cliproxyapi');
  await expect(cli).toHaveAttribute('data-state', 'ok');
  await expect(cli).toContainText('CLIProxyAPI');
  await expect(cli).toContainText("Shows how much of each AI account's limit is used");
  await expect(cli.getByTestId('integration-switch')).toHaveAttribute('aria-checked', 'true');
  await expect(cli.getByTestId('integration-switch')).toHaveAccessibleName('Turn on CLIProxyAPI');
  await expect(cli.getByTestId('integration-settings')).toHaveAccessibleName('CLIProxyAPI settings');
  await expect(cli.getByTestId('integration-status')).toHaveCount(0);
  await expect(cli.getByTestId('preset')).toHaveCount(0);
  for (const id of ['this_pc', 'volume', 'media']) {
    await expect(row(page, id).getByTestId('integration-switch')).toHaveAttribute('aria-checked', 'true');
    await expect(row(page, id).getByTestId('integration-settings')).toHaveCount(0);
  }
  await expect(row(page, 'this_pc')).toContainText("Shows this PC's CPU, memory, GPU, disk and network use.");
  await expect(page.getByText('Turn on the apps and services your Tiles use. Add their Tiles from Layout.')).toBeVisible();
});

test('a switch turns an Integration off and on at once, writing only enabled', async ({ page }) => {
  await openIntegrations(page);
  const before = await record(page);
  const toggle = row(page, 'cliproxyapi').getByTestId('integration-switch');
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-checked', 'false');
  await expect.poll(async () => (await record(page)).settings.enabled).toBe(false);
  const after = await record(page);
  expect(after.configPuts).toBe(before.configPuts);
  expect(after.enabledPuts).toBe(before.enabledPuts + 1);
  expect(after.settings).toMatchObject({ url: before.settings.url, interval_s: before.settings.interval_s });
  expect(after.keySet).toBe(true);
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-checked', 'true');
  await expect.poll(async () => (await record(page)).settings.enabled).toBe(true);

  const volume = row(page, 'volume').getByTestId('integration-switch');
  await volume.click();
  await expect(volume).toHaveAttribute('aria-checked', 'false');
  await page.getByRole('button', { name: 'Home', exact: true }).click();
  await page.getByRole('button', { name: 'Integrations', exact: true }).click();
  await expect(row(page, 'volume').getByTestId('integration-switch')).toHaveAttribute('aria-checked', 'false');
});

test('a failing Integration says so under its name, with Details on its settings page', async ({ page }) => {
  await openIntegrations(page);
  await page.evaluate(() => window.touchdeckDemo?.setHub(false));
  const status = row(page, 'cliproxyapi').getByTestId('integration-status');
  await expect(status).toHaveText("Can't connect");
  await expect(status).toHaveAttribute('data-tone', 'bad');
  await row(page, 'cliproxyapi').getByTestId('integration-details').click();
  await expect(page.getByTestId('integration-detail')).toHaveAttribute('data-id', 'cliproxyapi');
  await expect(page.getByTestId('detail-status')).toHaveText("Can't connect");
});

test('turning on an Integration that is not set up opens its setup and turns it on once setup is saved', async ({ page }) => {
  await openIntegrations(page, '?integrations=new');
  const cli = row(page, 'cliproxyapi');
  await expect(cli).toHaveAttribute('data-state', 'not_configured');
  await expect(cli.getByTestId('integration-switch')).toHaveAttribute('aria-checked', 'false');
  await cli.getByTestId('integration-switch').click();
  await expect(page.getByTestId('integration-detail')).toHaveAttribute('data-id', 'cliproxyapi');
  // The address comes filled in, so setup starts at the key it still needs.
  await expect(field(page, 'url')).not.toHaveValue('');
  await expect(page.getByTestId('secret-input')).toBeFocused();
  const header = page.getByTestId('detail-switch');
  await expect(header).toHaveAttribute('aria-checked', 'true');
  await expect(header).toHaveAttribute('aria-busy', 'true');
  await expect(page.getByTestId('detail-status')).toHaveText('Finish setup');
  await expect(page.getByTestId('detail-status')).toHaveAttribute('data-tone', 'wait');
  await expect(page.getByTestId('detail-checks')).toContainText('Finish setup to check.');
  expect((await record(page)).settings.enabled).toBe(false);

  await page.getByTestId('secret-input').fill(KEY);
  await page.getByTestId('secret-save').click();
  await expect(page.getByTestId('secret-state')).toHaveText('Set');
  await expect.poll(async () => (await record(page)).settings.enabled).toBe(true);
  await expect(header).toHaveAttribute('aria-checked', 'true');
  await expect(header).not.toHaveAttribute('aria-busy', 'true');
  await expect(page.getByTestId('detail-status')).toHaveAttribute('data-tone', 'ok');
  await page.getByTestId('detail-back').click();
  await expect(row(page, 'cliproxyapi').getByTestId('integration-switch')).toHaveAttribute('aria-checked', 'true');
});

test('flipping the switch off again undoes an enable that waits for setup', async ({ page }) => {
  await openIntegrations(page, '?integrations=new');
  await row(page, 'cliproxyapi').getByTestId('integration-switch').click();
  const header = page.getByTestId('detail-switch');
  await expect(header).toHaveAttribute('aria-busy', 'true');
  await header.click();
  await expect(header).toHaveAttribute('aria-checked', 'false');
  await expect(header).not.toHaveAttribute('aria-busy', 'true');
  await expect(page.getByTestId('detail-status')).toHaveCount(0);
  await expect(page.getByTestId('detail-checks')).toContainText('Turn on CLIProxyAPI to check.');
  await page.getByTestId('secret-input').fill(KEY);
  await page.getByTestId('secret-save').click();
  await expect(page.getByTestId('secret-state')).toHaveText('Set');
  await expect(header).toHaveAttribute('aria-checked', 'false');
  expect((await record(page)).settings.enabled).toBe(false);
});

test('backing out of an unfinished setup leaves the switch off', async ({ page }) => {
  await openIntegrations(page, '?integrations=new');
  await row(page, 'cliproxyapi').getByTestId('integration-switch').click();
  await expect(page.getByTestId('detail-switch')).toHaveAttribute('aria-checked', 'true');
  await page.getByTestId('detail-back').click();
  await expect(row(page, 'cliproxyapi').getByTestId('integration-switch')).toHaveAttribute('aria-checked', 'false');
  await expect(row(page, 'cliproxyapi').getByTestId('integration-settings')).toBeFocused();
  await row(page, 'cliproxyapi').getByTestId('integration-settings').click();
  await page.getByTestId('secret-input').fill(KEY);
  await page.getByTestId('secret-save').click();
  await expect(page.getByTestId('secret-state')).toHaveText('Set');
  await expect(page.getByTestId('detail-switch')).toHaveAttribute('aria-checked', 'false');
  expect((await record(page)).settings.enabled).toBe(false);
});

test('setting up with a key shows it only as Set, and never keeps it in the page', async ({ page }) => {
  await openCliProxy(page, '?integrations=new');
  await field(page, 'url').fill('https://hub.example.com/');
  await field(page, 'interval_s').fill('120');
  await expect(page.getByTestId('config-dirty')).toHaveText('Unsaved changes');
  await page.getByTestId('config-save').click();
  await expect(outcome(page, 'settings')).toHaveAttribute('data-event', 'saved');
  await expect(outcome(page, 'settings')).toHaveText('Saved');
  await expect(field(page, 'url')).toHaveValue('https://hub.example.com');
  await expect(page.getByTestId('config-save')).toHaveAttribute('aria-disabled', 'true');

  const input = page.getByTestId('secret-input');
  await expect(input).toHaveAttribute('type', 'password');
  await input.fill(KEY);
  await page.getByTestId('secret-save').click();
  await expect(page.getByTestId('secret-state')).toHaveText('Set');
  await expect(page.getByTestId('secret-field')).toHaveAttribute('data-set', 'true');
  await expect(input).toHaveCount(0);

  expect(await page.content()).not.toContain(KEY);
  const kept = await page.evaluate(() => JSON.stringify({ local: { ...localStorage }, session: { ...sessionStorage }, url: location.href }));
  expect(kept).not.toContain(KEY);
  expect(await record(page)).toMatchObject({ keySet: true, settings: { enabled: false, url: 'https://hub.example.com', interval_s: 120 } });
  expect(JSON.stringify(await record(page))).not.toContain(KEY);

  await page.getByTestId('secret-replace').click();
  await expect(page.getByTestId('secret-input')).toHaveValue('');
  await page.getByTestId('secret-cancel').click();
  await expect(page.getByTestId('secret-state')).toHaveText('Set');
});

test('Save sits under the settings it saves, apart from the key, and never sends enabled', async ({ page }) => {
  await openCliProxy(page);
  const settings = page.getByTestId('detail-settings');
  await expect(settings).toContainText('Connection');
  await expect(settings.getByTestId('config-save')).toBeVisible();
  await expect(settings.getByTestId('secret-field')).toHaveCount(0);
  await expect(page.getByTestId('detail-secrets').getByTestId('secret-field')).toBeVisible();
  const save = await settings.getByTestId('config-save').boundingBox();
  const url = await field(page, 'url').boundingBox();
  const key = await page.getByTestId('secret-field').boundingBox();
  expect(save!.y).toBeGreaterThan(url!.y);
  expect(save!.y + save!.height).toBeLessThan(key!.y);

  // The demo answers the API inside the page, so the spec listens where the page sends.
  await page.evaluate(() => {
    const send = window.fetch;
    const seen: unknown[] = [];
    Object.assign(window, { configBodies: seen });
    window.fetch = (input, init) => {
      const url = input instanceof Request ? input.url : String(input);
      if (init?.method === 'PUT' && url.endsWith('/integrations/cliproxyapi/config')) seen.push(JSON.parse(String(init.body)));
      return send(input, init);
    };
  });
  await field(page, 'interval_s').fill('600');
  await page.getByTestId('config-save').click();
  await expect(outcome(page, 'settings')).toHaveAttribute('data-event', 'saved');
  expect(await page.evaluate(() => (window as unknown as { configBodies: unknown[] }).configBodies)).toEqual([{ url: 'https://hub.example.com', interval_s: 600 }]);
});

test('unsaved edits survive the switch', async ({ page }) => {
  await openCliProxy(page);
  await field(page, 'interval_s').fill('900');
  await page.getByTestId('detail-switch').click();
  await expect(page.getByTestId('detail-switch')).toHaveAttribute('aria-checked', 'false');
  await expect.poll(async () => (await record(page)).settings.enabled).toBe(false);
  await expect(field(page, 'interval_s')).toHaveValue('900');
  await expect(page.getByTestId('config-dirty')).toBeVisible();
  await expect(page.getByTestId('detail-status')).toHaveCount(0);
});

test('checks fields inline and warns before a key travels unencrypted', async ({ page }) => {
  await openCliProxy(page);
  const before = (await record(page)).configPuts;
  await field(page, 'interval_s').fill('soon');
  await page.getByTestId('config-save').click();
  await expect(page.getByTestId('field-error')).toHaveText('Enter a whole number.');
  await expect(field(page, 'interval_s')).toHaveAttribute('aria-invalid', 'true');
  expect((await record(page)).configPuts).toBe(before);
  await field(page, 'interval_s').fill('300');
  await expect(page.getByTestId('field-error')).toHaveCount(0);

  await field(page, 'url').fill('http://192.168.1.20:8317');
  await expect(page.getByTestId('transport-warning')).toHaveText("This address isn't on this PC and uses http://, so the key is sent unencrypted. Use https://.");
  await field(page, 'url').fill('http://hub.example.com');
  await expect(page.getByTestId('transport-warning')).toHaveText("This address uses http:// and isn't on your network, so the key isn't sent. Use https://.");
  await field(page, 'url').fill('http://127.0.0.1:8317');
  await expect(page.getByTestId('transport-warning')).toHaveCount(0);
});

test('the status box follows the hub and the switch', async ({ page }) => {
  await openCliProxy(page);
  const status = page.getByTestId('detail-status');
  await expect(status).toHaveText(/^2 accounts · checked 1[2-9] s ago$/);
  await expect(status).toHaveAttribute('data-tone', 'ok');

  await page.evaluate(() => window.touchdeckDemo?.setHub(false));
  await expect(status).toHaveText("Can't connect");
  await expect(status).toHaveAttribute('data-tone', 'bad');
  await page.getByText('Details', { exact: true }).first().click();
  await expect(page.getByTestId('detail-status-detail')).toHaveText('connection refused');

  await page.evaluate(() => window.touchdeckDemo?.deliver({ device: 'mock0002', sent: 0, error: 'Device is busy' }));
  await expect(page.getByTestId('delivery-error')).toHaveAttribute('data-device', 'mock0002');
  await expect(page.getByTestId('delivery-error')).toContainText("Couldn't send values to");

  await page.evaluate(() => window.touchdeckDemo?.setHub(true));
  await expect(status).toHaveAttribute('data-tone', 'ok');
  await page.getByTestId('detail-switch').click();
  await expect(status).toHaveCount(0);
  await page.getByTestId('detail-back').click();
  await expect(row(page, 'cliproxyapi').getByTestId('integration-switch')).toHaveAttribute('aria-checked', 'false');
});

test('Check now asks the hub once, then waits out the floor politely', async ({ page }) => {
  await openCliProxy(page);
  const refresh = page.getByTestId('refresh');
  await expect(refresh).toHaveText('Check now');
  await refresh.click();
  await expect(outcome(page, 'status')).toHaveAttribute('data-event', 'refreshed');
  await expect(outcome(page, 'status')).toHaveText('Check started');
  await expect(refresh).toHaveAttribute('data-waiting', 'true');
  await expect(refresh).toHaveAttribute('aria-disabled', 'true');
  await refresh.click({ force: true });
  expect((await record(page)).refreshes).toBe(1);
  await expect(page.getByText('Checks are at least 15 s apart, because each one queries your AI services.')).toBeVisible();
});

test('moving the hub to another origin forgets the key and says so', async ({ page }) => {
  await openCliProxy(page);
  await expect(page.getByTestId('secret-field')).toHaveAttribute('data-set', 'true');
  await field(page, 'url').fill('https://hub.example.com/cliproxy');
  await page.getByTestId('config-save').click();
  await expect(outcome(page, 'settings')).toHaveAttribute('data-event', 'saved');
  await expect(page.getByTestId('secret-field')).toHaveAttribute('data-set', 'true');

  await field(page, 'url').fill('https://other-hub.example.com');
  await page.getByTestId('config-save').click();
  await expect(outcome(page, 'settings')).toHaveAttribute('data-event', 'secret-forgotten');
  await expect(outcome(page, 'settings')).toHaveText('Saved. The address changed, so the management key was forgotten. Enter it again.');
  await expect(page.getByTestId('secret-field')).toHaveAttribute('data-set', 'false');
  await expect(page.getByTestId('secret-input')).toHaveAttribute('placeholder', 'Not set');
  await expect(page.getByTestId('detail-status')).toHaveText('Finish setup');
  expect((await record(page)).keySet).toBe(false);
});

test('Forget asks first, then removes the key', async ({ page }) => {
  await openCliProxy(page);
  await page.getByTestId('secret-forget').click();
  const dialog = page.getByTestId('forget-dialog');
  await expect(dialog).toContainText('Forget the management key?');
  await expect(dialog).toContainText('CLIProxyAPI stops until you add it again.');
  await page.getByTestId('forget-confirm').click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId('secret-field')).toHaveAttribute('data-set', 'false');
  expect((await record(page)).keySet).toBe(false);
  await page.getByTestId('detail-back').click();
  await expect(row(page, 'cliproxyapi').getByTestId('integration-status')).toHaveText('Finish setup');
  await expect(row(page, 'cliproxyapi').getByTestId('integration-switch')).toHaveAttribute('aria-checked', 'true');
});

test('shows every account with its limits, as the AI usage page did', async ({ browserName, context, page }) => {
  if (browserName === 'chromium') await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await openCliProxy(page);
  const accounts = page.getByTestId('usage-account');
  await expect(accounts).toHaveCount(2);
  const codex = page.locator('[data-testid="usage-account"][data-account="codex:Work"]');
  await expect(codex.getByTestId('usage-plan')).toHaveText('pro');
  await expect(codex.getByTestId('usage-five-hour')).toContainText('42%');
  await expect(codex.getByTestId('usage-five-hour')).toContainText('Resets in 2h 10m');
  await expect(codex.getByTestId('usage-seven-day')).toContainText('10%');
  await expect(codex.getByTestId('usage-status')).toContainText('Active');
  await expect(codex.getByTestId('usage-status')).toContainText('128 OK · 2 failed · 6 in 10 min');
  await expect(page.locator('[data-testid="usage-account"][data-account="claude:Personal"]').getByTestId('usage-seven-day')).toContainText('—');

  await codex.getByTestId('usage-copy').click();
  await expect(codex.getByTestId('usage-copy')).toHaveText('Copied');
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('codex:Work');
});

test('Add Tile opens on Integrations, with real previews by Integration, and places one where it points', async ({ page }) => {
  await startOnShortcuts(page);
  await expect(page.getByTestId('add-tab-integrations')).toHaveAttribute('aria-selected', 'true');
  const section = page.getByTestId('add-presets');
  await expect(section).toBeVisible();
  const group = section.locator('[data-testid="preset-group"][data-id="cliproxyapi"]');
  await expect(group).toContainText('CLIProxyAPI');
  await expect(group.getByTestId('preset-group-settings')).toBeVisible();
  await expect(section.locator('[data-testid="preset-group"][data-id="this_pc"]').getByTestId('preset-group-settings')).toHaveCount(0);
  await expect(group.getByTestId('preset')).toHaveCount(9);
  const five = group.locator('[data-preset="five_hour_pct"]');
  await expect(five).toHaveAccessibleName('Add 5-hour used');
  await expect(five.getByTestId('tile-level').or(five.locator('.scene-text')).first()).toBeAttached();
  await expect(five).toContainText('42%');
  await five.click();
  const placed = page.locator('[data-testid="tile"][data-row="2"][data-col="2"]');
  await expect(placed).toHaveAttribute('aria-label', '5-hour used');
  await expect(placed).toHaveAttribute('aria-pressed', 'true');
  await expect(section).toHaveCount(0);
});

test("Add Tile keeps the tab chosen by hand, and arrow keys move between tabs", async ({ page }) => {
  await startOnShortcuts(page);
  const actions = page.getByTestId('add-tab-actions');
  const integrations = page.getByTestId('add-tab-integrations');
  await expect(integrations).toBeFocused();
  await page.keyboard.press('ArrowLeft');
  await expect(actions).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByTestId('add-kind').first()).toBeVisible();
  await expect(page.getByTestId('add-presets')).toBeHidden();
  await page.getByTestId('add-cancel').click();
  await page.locator('[data-testid="empty-cell"][data-row="2"][data-col="3"]').click();
  await expect(actions).toHaveAttribute('aria-selected', 'true');
});

test('a Preset the Device draws without its graph is previewed as the Tile it places', async ({ page }) => {
  await startOnShortcuts(page);
  const cpu = page.locator('[data-testid="preset-group"][data-id="this_pc"] [data-preset="cpu_pct"]');
  await expect(cpu).toHaveAttribute('data-look', 'bar');
  await expect(cpu.getByTestId('tile-graph')).toHaveCount(0);
  await expect(cpu).toContainText('23%');
  const volume = page.locator('[data-testid="preset-group"][data-id="volume"] [data-preset="volume_bar"]');
  await expect(volume).toHaveAttribute('data-needs', /Firmware/);
});

test('Add Tile points to Integrations when none offers Tiles', async ({ page }) => {
  await openIntegrations(page, '?integrations=new');
  for (const id of ['this_pc', 'volume', 'media']) {
    await row(page, id).getByTestId('integration-switch').click();
    await expect(row(page, id).getByTestId('integration-switch')).toHaveAttribute('aria-checked', 'false');
  }
  await page.getByRole('button', { name: 'Layout', exact: true }).click();
  await page.getByTestId('start-starter').click();
  await page.getByTestId('page-tab').filter({ hasText: 'Shortcuts' }).click();
  await page.locator('[data-testid="empty-cell"][data-row="2"][data-col="2"]').click();
  await expect(page.getByTestId('add-tab-actions')).toHaveAttribute('aria-selected', 'true');
  await page.getByTestId('add-tab-integrations').click();
  const section = page.getByTestId('add-presets');
  await expect(section.getByTestId('add-presets-empty')).toContainText('Turn on an Integration to add its Tiles.');
  await expect(section.getByTestId('add-presets-closing')).toHaveCount(0);
  await section.getByTestId('add-presets-setup').click();
  await expect(page.getByTestId('integration-list')).toBeVisible();
});

test('focus is visible on the list and on every control of the CLIProxyAPI page', async ({ page }) => {
  await openIntegrations(page);
  for (const id of ['integration-settings', 'integration-switch']) {
    expect({ id, ...(await reach(page, id)) }).toEqual({ id, visible: true, ring: true });
  }
  await row(page, 'cliproxyapi').getByTestId('integration-settings').click();
  await expect(page.getByTestId('usage-account')).toHaveCount(2);
  for (const id of ['detail-back', 'detail-switch', 'config-field', 'secret-replace', 'secret-forget', 'refresh', 'usage-copy']) {
    expect({ id, ...(await reach(page, id)) }).toEqual({ id, visible: true, ring: true });
  }
});
