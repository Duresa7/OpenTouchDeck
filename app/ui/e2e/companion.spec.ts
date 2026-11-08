import { join } from 'node:path';
import { expect, test, type APIRequestContext, type Locator, type Page, type Response } from '@playwright/test';
import type { ActionLog, AllowedAction, ApprovalRequest, NewBoard } from '../src/lib/companion/api.ts';
import type { Layout } from '../src/lib/layout/model.ts';
import { exportText, menu, openLayout, sorted, status, tile, waitEvent } from './layout-helpers.ts';

test.describe.configure({ mode: 'serial' });

async function companion(request: APIRequestContext) {
  const minted = await request.post('/__companion/session', { headers: { 'X-TouchDeck-Dev': '1' } });
  expect(minted.status()).toBe(200);
  const { token } = (await minted.json()) as { token: string };
  return (method: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE', path: string, data?: unknown) =>
    request.fetch(`/api/v1${path}`, { method, data, headers: { Authorization: `Bearer ${token}` } });
}

const layout = {
  schema: 1, name: 'E2E', orientation: 'landscape',
  pages: [
    { id: 'keys', title: 'Keys', kind: 'grid', rows: 2, cols: 2, tiles: [
      { id: 'copy', label: 'Copy', row: 0, col: 0, rowSpan: 1, colSpan: 1, icon: null, action: { type: 'none' } },
    ] },
    { id: 'more', title: 'More', kind: 'grid', rows: 1, cols: 1, tiles: [] },
  ],
};

test('Home lists the mock Devices from the real Companion and opens one', async ({ page }) => {
  const session = page.waitForResponse((r) => r.url().endsWith('/__companion/session'));
  const events = page.waitForEvent('websocket', (ws) => new URL(ws.url()).pathname === '/api/v1/events');
  await page.goto('/');

  expect((await session).status()).toBe(200);
  await events;

  const cards = page.getByRole('list', { name: 'Devices' }).getByRole('listitem');
  await expect(cards).toHaveCount(3);
  for (let i = 0; i < 3; i++) {
    await expect(cards.nth(i)).toContainText(`Mock Device mock000${i + 1}`);
    await expect(cards.nth(i).getByTestId('device-status')).toHaveAccessibleName(/: Connected/);
    await expect(cards.nth(i).locator('.device-spill')).toHaveCount(1);
  }
  const widths = await cards.evaluateAll((els) => els.map((el) => el.getBoundingClientRect().width));
  expect(widths).toEqual([312, 312, 312]);

  await cards.nth(2).getByRole('button').press('Enter');
  await expect(page.locator('main')).toHaveAttribute('data-device-id', 'mock0003');
});

test('a Layout pushed to a Device shows on its miniature, and stays when it is unplugged', async ({ page, request }) => {
  const api = await companion(request);
  await page.goto('/');
  const card = page.getByRole('list', { name: 'Devices' }).getByRole('listitem').nth(1);
  await expect(card.getByTestId('device-status')).toHaveAccessibleName(/: Connected/);
  await expect(card.locator('.device-scene')).not.toHaveAttribute('data-page');
  await page.evaluate(() => { (window as unknown as { notReloaded: boolean }).notReloaded = true; });

  const pushed = await api('PUT', '/devices/mock0002/layouts/landscape', layout);
  expect(pushed.status()).toBe(202);
  await expect(card.locator('.device-scene')).toHaveAttribute('data-page', 'keys');
  await expect(card.locator('.device-scene')).toContainText('Keys');
  await expect(card.locator('.device-scene')).toContainText('Copy');
  expect(await page.evaluate(() => (window as unknown as { notReloaded?: boolean }).notReloaded)).toBe(true);

  expect((await api('POST', '/test/ports', { unplug: 'mock0002' })).status()).toBe(204);
  await expect(card).toContainText('Not connected');
  await expect(card).toContainText('Last seen');
  await expect(card.locator('.device-spill')).toHaveCSS('opacity', '0');
  await expect(card.locator('.device-scene')).toHaveAttribute('data-page', 'keys');
  await expect(card.locator('.device-scene')).toContainText('Copy');
  if (process.env.TD_SHOTS) await page.screenshot({ path: join(process.env.TD_SHOTS, 'home-companion-3.png') });
});

type Api = Awaited<ReturnType<typeof companion>>;

async function stored(api: Api, device: string): Promise<Layout | null> {
  const record = await api('GET', `/test/devices/${device}/record`);
  expect(record.status()).toBe(200);
  return ((await record.json()) as { layouts: { landscape: Layout | null } }).layouts.landscape;
}

async function pushElsewhere(api: Api, device: string, body: unknown) {
  const pushed = await api('PUT', `/devices/${device}/layouts/landscape`, body);
  expect(pushed.status()).toBe(202);
  const { id } = (await pushed.json()) as { id: string };
  await expect.poll(async () => ((await (await api('GET', `/jobs/${id}`)).json()) as { state: string }).state, { timeout: 30_000 }).toBe('succeeded');
}

test('the Layout screen sends a Layout to a mock Device, and the Device shows it', async ({ page, request }) => {
  const api = await companion(request);
  await openLayout(page);
  await expect(page.getByTestId('device-name')).toHaveText('Mock Device mock0001');
  await page.getByTestId('start-starter').click();
  await page.getByTestId('push').click();
  const sent = await waitEvent(page, 'pushed', 30_000);
  await expect(sent).toHaveText('Sent to Mock Device mock0001');
  await expect(sent).toHaveAttribute('data-pages', '2');
  await expect(sent).toHaveAttribute('data-tiles', '16');
  await expect(page.getByTestId('save-state')).toHaveAttribute('data-state', 'saved');
  const mine = JSON.parse(await exportText(page)) as Layout;
  expect(sorted((await stored(api, 'mock0001'))!)).toEqual(sorted(mine));
  await page.getByRole('button', { name: 'Home', exact: true }).click();
  const card = page.getByRole('list', { name: 'Devices' }).getByRole('listitem').first();
  await expect(card.locator('.device-scene')).toHaveAttribute('data-page', 'media');
  if (process.env.TD_SHOTS) await page.screenshot({ path: join(process.env.TD_SHOTS, 'layout-companion-pushed.png') });
});

test('a send after the mock Device changed offers Keep mine, and Load from replaces after a confirm', async ({ page, request }) => {
  const api = await companion(request);
  await openLayout(page);
  await page.getByTestId('start-pull').click();
  await expect(await waitEvent(page, 'pulled', 30_000)).toHaveText('Loaded “Starter” from Mock Device mock0001');
  await tile(page, 'prev').click();
  await page.getByTestId('tile-clear').click();

  await pushElsewhere(api, 'mock0001', layout);
  await page.getByTestId('push').click();
  const dialog = page.getByRole('alertdialog');
  await expect(dialog).toContainText('Mock Device mock0001 has “E2E”', { timeout: 30_000 });
  await expect(dialog).toContainText('(2 Pages, 1 Tile). Replace the landscape Layout here with it?');
  await dialog.getByRole('button', { name: 'Keep mine' }).click();
  await expect(await waitEvent(page, 'pushed', 30_000)).toHaveAttribute('data-tiles', '15');
  expect((await stored(api, 'mock0001'))?.name).toBe('Starter');

  await pushElsewhere(api, 'mock0001', layout);
  await menu(page, 'pull');
  await dialog.getByRole('button', { name: 'Replace' }).click();
  await expect(await waitEvent(page, 'pulled', 30_000)).toHaveText('Loaded “E2E” from Mock Device mock0001');
  expect(sorted(JSON.parse(await exportText(page)) as Layout)).toEqual(sorted((await stored(api, 'mock0001'))!));
  await expect(status(page)).toHaveAttribute('data-kind', 'ok');
});

interface MockRecord {
  settings: { orientation: string };
  reboots: number;
}

test('the Device panel renames a mock Device, switches its orientation with a restart, and restarts it', async ({ page, request }) => {
  const api = await companion(request);
  const record = async (): Promise<MockRecord> => (await api('GET', '/test/devices/mock0003/record')).json();
  const before = await record();
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.getByRole('button', { name: 'Edit Mock Device mock0003' }).click();
  await expect(page.getByTestId('editor')).toHaveAttribute('data-device-id', 'mock0003');
  await page.getByTestId('device-name').click();
  const status = page.getByTestId('device-panel-status');
  const name = page.getByTestId('device-panel-name');
  await expect(name).toBeEditable();

  await name.fill('Bench');
  await name.press('Enter');
  await expect(status).toHaveAttribute('data-event', 'saved');
  await expect(page.getByTestId('device-name')).toHaveText('Bench');
  const renamed: { name: string } = await (await api('GET', '/devices/mock0003')).json();
  expect(renamed.name).toBe('Bench');
  await expect(page.getByTestId('device-panel-brightness')).toHaveCount(0);

  await page.getByTestId('device-panel-portrait').click();
  await page.getByTestId('device-panel-confirm-restart').click();
  await expect(status).toHaveAttribute('data-event', 'restarting');
  await expect(status).not.toHaveAttribute('data-event', 'restarting', { timeout: 30_000 });
  await expect(status).not.toHaveAttribute('data-kind', 'error');
  await expect(page.getByTestId('device-panel-portrait')).toHaveAttribute('aria-pressed', 'true');
  const switched = await record();
  expect(switched.settings.orientation).toBe('portrait');
  expect(switched.reboots).toBe(before.reboots + 1);

  await page.getByTestId('device-panel-restart').click();
  await expect(status).toHaveAttribute('data-event', 'restarting');
  await expect(status).not.toHaveAttribute('data-event', 'restarting', { timeout: 30_000 });
  await expect(status).not.toHaveAttribute('data-kind', 'error');
  expect((await record()).reboots).toBe(before.reboots + 2);

  const restored = await api('PATCH', '/devices/mock0003/settings', { idle_timeout_s: null, orientation: 'landscape', name: 'Mock Device mock0003', brightness: null });
  expect(restored.status()).toBe(200);
});

test('the Device panel turns on the photo frame of a mock Device, which stores it', async ({ page, request }) => {
  const api = await companion(request);
  const frame = async (): Promise<unknown> => (await (await api('GET', '/devices/mock0003/settings')).json()).photo_frame;
  expect(await frame()).toEqual({ enabled: false, timeout_s: 60, interval_s: 15, folder: null });
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.getByRole('button', { name: 'Edit Mock Device mock0003' }).click();
  await expect(page.getByTestId('editor')).toHaveAttribute('data-device-id', 'mock0003');
  await page.getByTestId('device-name').click();
  const photo = page.getByTestId('device-panel-photo-frame');
  await expect(photo).toBeEnabled();
  await photo.click();
  await expect(page.getByTestId('device-panel-status')).toHaveAttribute('data-event', 'saved');
  await page.getByTestId('device-panel-photo-frame-after').selectOption({ label: '5 min' });
  await expect.poll(frame).toEqual({ enabled: true, timeout_s: 300, interval_s: 15, folder: null });
  await expect(page.getByTestId('device-panel-folder-choose')).toHaveCount(0);
  await expect(page.getByTestId('device-panel-folder-hint')).toHaveText('Choose a folder in the Open TouchDeck app.');

  const restored = await api('PATCH', '/devices/mock0003/settings', { photo_frame: { enabled: false, timeout_s: 60 } });
  expect(restored.status()).toBe(200);
});

const NO_SUCH_PROGRAM = 'touchdeck-e2e-no-such-program';
const RLO_ARG = 'report‮gpj.exe';
const RLO_SHOWN = 'report\\u{202E}gpj.exe';

const actionsLayout = {
  schema: 1, name: 'Actions', orientation: 'landscape',
  pages: [
    { id: 'main', title: 'Main', kind: 'grid', rows: 2, cols: 2, tiles: [
      { id: 'run', label: 'Run', row: 0, col: 0, rowSpan: 1, colSpan: 1, icon: null, action: { type: 'command', program: NO_SUCH_PROGRAM, args: [RLO_ARG], cwd: '', shell: false } },
      { id: 'copy', label: 'Copy', row: 0, col: 1, rowSpan: 1, colSpan: 1, icon: null, action: { type: 'key', usage: 6, mods: ['ctrl'] } },
    ] },
  ],
};

const called = (method: string, route: RegExp) => (r: Response) => r.request().method() === method && route.test(new URL(r.url()).pathname);

async function call(api: Api, method: 'POST' | 'DELETE', path: string, data?: unknown) {
  expect((await api(method, path, data)).status()).toBe(204);
}

const pressRun = (api: Api) => call(api, 'POST', '/test/devices/mock0001/press', { page: 'main', tile: 'run', kind: 'command' });
const answer = (api: Api, answers: string[]) => call(api, 'POST', '/test/approvals/script', { answers });
const emitKeys = (api: Api, events: Record<string, unknown>[]) => call(api, 'POST', '/test/devices/mock0001/emit', { events: events.map((event) => ({ m: 'key.report', ...event })) });

async function expectSuppressedOnceTheDialogShows(api: Api, reports: Locator) {
  await expect(async () => {
    const before = await reports.count();
    await emitKeys(api, [{ type: 'con', usage: 233 }]);
    await expect(reports).toHaveCount(before + 1, { timeout: 1_000 });
    await expect(reports.last()).toHaveAttribute('data-outcome', 'suppressed', { timeout: 100 });
  }).toPass();
}

async function forgetGrants(api: Api) {
  const allowed: AllowedAction[] = await (await api('GET', '/actions/allowed')).json();
  for (const grant of allowed) if (grant.program === NO_SUCH_PROGRAM) await call(api, 'DELETE', `/actions/allowed/${grant.hash}`);
}

async function openActions(page: Page, api: Api) {
  await forgetGrants(api);
  await pushElsewhere(api, 'mock0001', actionsLayout);
  await openLayout(page);
  await page.getByTestId('start-pull').click();
  await waitEvent(page, 'pulled', 30_000);
}

test('a pressed command Tile waits for approval on this PC with its arguments escaped, and clears when the dialog answers', async ({ page, request }) => {
  const api = await companion(request);
  await openActions(page, api);
  await tile(page, 'run').click();
  await expect(page.getByTestId('command-line')).toHaveText(`${NO_SUCH_PROGRAM} ${RLO_SHOWN}`);

  await pressRun(api);
  const pending = page.getByTestId('approval-pending');
  await expect(pending).toContainText('Waiting for approval on this PC');
  await expect(pending).toContainText('Run on the Main Page of Mock Device mock0001');
  await expect(pending.getByTestId('approval-program')).toHaveText(NO_SUCH_PROGRAM);
  await expect(pending.getByTestId('approval-args').locator('li')).toHaveText([RLO_SHOWN]);
  expect(await pending.textContent()).not.toContain('‮');
  await expect(pending.getByRole('button')).toHaveCount(1);

  const listed = page.waitForResponse(called('GET', /^\/api\/v1\/approvals$/));
  await openLayout(page);
  const requests: ApprovalRequest[] = await (await listed).json();
  expect(requests.map((r) => ({ tile: r.tile, args: r.args }))).toEqual([{ tile: 'run', args: [RLO_ARG] }]);
  await expect(pending).toBeVisible();

  const shown = page.waitForResponse(called('POST', /^\/api\/v1\/approvals\/[^/]+\/show$/));
  await pending.getByTestId('approval-show').click();
  expect((await shown).status()).toBe(204);

  await answer(api, ['deny']);
  await expect(pending).toHaveCount(0);
  await tile(page, 'run').click();
  await page.getByTestId('action-log-panel').locator('summary').click();
  await expect(page.getByTestId('action-entry').first()).toHaveAttribute('data-status', 'denied');
});

test('Always allow lists the action, the Action log reads and follows the presses, and Remove makes the next press ask', async ({ page, request }) => {
  const api = await companion(request);
  await openActions(page, api);
  await tile(page, 'run').click();
  await page.getByTestId('action-log-panel').locator('summary').click();

  await answer(api, ['always']);
  await pressRun(api);
  const allowed = page.getByTestId('allowed-action');
  await expect(allowed).toHaveCount(1);
  await expect(allowed).toHaveAttribute('data-mine', 'true');
  await expect(allowed.locator('.command-line')).toHaveText(`${NO_SUCH_PROGRAM} ${RLO_SHOWN}`);
  expect(await allowed.textContent()).not.toContain('‮');
  await expect(page.getByTestId('action-entry').first()).toHaveAttribute('data-status', 'failed');
  await expect(page.getByTestId('approval-pending')).toHaveCount(0);

  const logged = page.waitForResponse(called('GET', /^\/api\/v1\/actions\/log$/));
  await openLayout(page);
  await tile(page, 'run').click();
  await page.getByTestId('action-log-panel').locator('summary').click();
  const log = await logged;
  expect(new URL(log.url()).searchParams.get('device')).toBe('mock0001');
  const entries: ActionLog[] = await log.json();
  expect(entries.at(-1)).toMatchObject({ tile: 'run', status: 'failed' });
  await expect(page.getByTestId('action-entry').first()).toHaveAttribute('data-status', 'failed');

  const removed = page.waitForResponse(called('DELETE', /^\/api\/v1\/actions\/allowed\/[^/]+$/));
  await page.getByTestId('allowed-remove').click();
  expect((await removed).status()).toBe(204);
  await expect(page.getByTestId('allowed-actions')).toHaveCount(0);
  expect(await (await api('GET', '/actions/allowed')).json()).toEqual([]);

  await pressRun(api);
  await expect(page.getByTestId('approval-pending')).toBeVisible();
  await answer(api, ['deny']);
  await expect(page.getByTestId('approval-pending')).toHaveCount(0);
  await expect(page.getByTestId('action-entry').first()).toHaveAttribute('data-status', 'denied');
});

test("Key test shows the mock Device's key reports from the key-mirror topic, suppressed while the dialog waits", async ({ page, request }) => {
  const api = await companion(request);
  const ops: string[] = [];
  page.on('websocket', (ws) => ws.on('framesent', ({ payload }) => typeof payload === 'string' && payload.includes('key-mirror') && ops.push(payload)));
  await openActions(page, api);
  await tile(page, 'copy').click();
  await page.getByTestId('key-test-panel').locator('summary').click();
  await expect.poll(() => ops.map((op): unknown => JSON.parse(op))).toContainEqual({ op: 'subscribe', topic: 'key-mirror', device: 'mock0001' });

  const reports = page.getByTestId('key-report');
  await expect(async () => {
    await emitKeys(api, [{ type: 'kbd', mods: 0, keys: [] }]);
    await expect(reports.first()).toBeVisible({ timeout: 1_000 });
  }).toPass();
  await page.getByTestId('key-monitor-clear').click();
  await expect(reports).toHaveCount(0);

  await emitKeys(api, [{ type: 'kbd', mods: 1, keys: [6] }, { type: 'kbd', mods: 0, keys: [] }]);
  await expect(reports).toHaveCount(2);
  await expect(reports.first()).toHaveAttribute('data-outcome', 'typed');

  await pressRun(api);
  await expect(page.getByTestId('approval-pending')).toBeVisible();
  await expectSuppressedOnceTheDialogShows(api, reports);
  await answer(api, ['deny']);
  await expect(page.getByTestId('approval-pending')).toHaveCount(0);

  await page.getByTestId('key-test-panel').locator('summary').click();
  await expect.poll(() => ops.map((op): unknown => JSON.parse(op))).toContainEqual({ op: 'unsubscribe', topic: 'key-mirror', device: 'mock0001' });
});

test('a board on a mock port shows a New board card that waits for a click, and leaves when unplugged', async ({ page, request }) => {
  const api = await companion(request);
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();

  const plugged = await api('POST', '/test/ports', { plug: { id: 'mock00ff', kind: 'other-firmware' } });
  expect(plugged.status()).toBe(204);
  const card = page.getByTestId('new-board');
  await expect(card).toHaveAttribute('data-step', 'detected', { timeout: 15_000 });
  await expect(card).toHaveAttribute('data-id', 'mock00ff');
  await expect(card).toContainText("doesn't run Open TouchDeck Firmware yet.");
  await expect(card.getByTestId('board-install')).toBeVisible();
  await expect(page.getByTestId('onboarding')).toHaveCount(0);

  const unplugged = await api('POST', '/test/ports', { unplug: 'mock00ff' });
  expect(unplugged.status()).toBe(204);
  await expect(card).toHaveCount(0, { timeout: 15_000 });
});

test('a name saved through settings_patch ends setup, and a ready Device never opens it', async ({ page, request }) => {
  const api = await companion(request);
  const named = await api('PATCH', '/devices/mock0003/settings', { idle_timeout_s: null, orientation: null, name: 'Shelf', brightness: null });
  expect(named.status()).toBe(200);
  const device: { name: string | null; setup: string } = await (await api('GET', '/devices/mock0003')).json();
  expect(device).toMatchObject({ name: 'Shelf', setup: 'ready' });

  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' }).getByRole('listitem').nth(2)).toContainText('Shelf');
  await expect(page.getByTestId('onboarding')).toHaveCount(0);

  const restored = await api('PATCH', '/devices/mock0003/settings', { idle_timeout_s: null, orientation: null, name: 'Mock Device mock0003', brightness: null });
  expect(restored.status()).toBe(200);
});

const consent = (api: Api, answers: ('accept' | 'decline')[]) => call(api, 'POST', '/test/firmware/script', { answers });

async function plugNewBoard(page: Page, api: Api, id: string) {
  await call(api, 'POST', '/test/ports', { plug: { id, kind: 'other-firmware' } });
  const card = page.getByTestId('new-board');
  await expect(card).toHaveAttribute('data-id', id, { timeout: 15_000 });
  await expect(card).toHaveAttribute('data-step', 'detected');
  return card;
}

async function reboots(api: Api, id: string): Promise<number> {
  const record = await api('GET', `/test/devices/${id}/record`);
  expect(record.status()).toBe(200);
  const { reboots }: MockRecord = await record.json();
  return reboots;
}

test('a New board installs once this PC accepts, comes up needing setup, and setup makes it ready', async ({ page, request }) => {
  const api = await companion(request);
  await consent(api, []);
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  const card = await plugNewBoard(page, api, 'mock00fe');

  await card.getByTestId('board-install').click();
  await expect(card).toHaveAttribute('data-step', 'waiting');
  await expect(card.getByTestId('board-status')).toHaveText('Confirm on this PC to continue.');
  expect(await reboots(api, 'mock00fe')).toBe(0);

  await consent(api, ['accept']);
  const dialog = page.getByTestId('onboarding');
  await expect(dialog).toHaveAttribute('data-device-id', 'mock00fe', { timeout: 30_000 });
  expect(await (await api('GET', '/devices/mock00fe')).json()).toMatchObject({ setup: 'needs-setup' });

  await expect(dialog).toHaveAttribute('data-step', 'name');
  await page.getByTestId('onboarding-name').fill('Workbench');
  await page.getByTestId('onboarding-next').click();
  await expect(dialog).toHaveAttribute('data-step', 'orientation');
  await page.getByTestId('onboarding-next').click();
  await expect(dialog).toHaveAttribute('data-step', 'layout');
  await page.getByTestId('onboarding-next').click();
  await expect(dialog).toHaveAttribute('data-step', 'done');
  await page.getByTestId('onboarding-done').click();
  await expect(dialog).toHaveCount(0);
  expect(await (await api('GET', '/devices/mock00fe')).json()).toMatchObject({ name: 'Workbench', setup: 'ready' });

  await call(api, 'POST', '/test/ports', { unplug: 'mock00fe' });
  await expect.poll(async () => (await api('DELETE', '/devices/mock00fe')).status(), { timeout: 15_000 }).toBe(204);
});

test('a declined install writes nothing, keeps the New board card, and opens no setup', async ({ page, request }) => {
  const api = await companion(request);
  await consent(api, []);
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  const card = await plugNewBoard(page, api, 'mock00fd');

  await card.getByTestId('board-install').click();
  await expect(card).toHaveAttribute('data-step', 'waiting');
  await consent(api, ['decline']);
  await expect(card).toHaveAttribute('data-step', 'declined');
  await expect(card.getByTestId('board-status')).toHaveText('Declined on this PC. Nothing changed on the board.');
  expect(await reboots(api, 'mock00fd')).toBe(0);
  const boards: NewBoard[] = await (await api('GET', '/new-boards')).json();
  expect(boards.map((b) => b.id)).toContain('mock00fd');
  expect((await api('GET', '/devices/mock00fd')).status()).toBe(404);
  await expect(page.getByTestId('onboarding')).toHaveCount(0);

  await card.getByTestId('board-cancel').click();
  await expect(card).toHaveAttribute('data-step', 'detected');
  await expect(card.getByTestId('board-install')).toBeVisible();
  await call(api, 'POST', '/test/ports', { unplug: 'mock00fd' });
  await expect(card).toHaveCount(0, { timeout: 15_000 });
});
