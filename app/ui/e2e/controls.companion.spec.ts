import { mkdirSync } from 'node:fs';
import { readFile, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test, type APIRequestContext, type Page } from '@playwright/test';
import type { ControlLayout } from '../src/lib/companion/api.ts';
import type { GridPage, Layout, Tile } from '../src/lib/layout/model.ts';
import { consoleErrors } from './break-helpers.ts';
import { waitEvent } from './layout-helpers.ts';

test.describe.configure({ mode: 'serial' });

const DEVICE = 'mock0001';
const NAME = 'Mock Device mock0001';
const PUSHED_COPY = fileURLToPath(new URL(`../../target/ui-e2e-companion/layouts/pushed-v1/${DEVICE}/landscape.json`, import.meta.url));
const shots = process.env.TD_SHOTS;

const CHECKING: ControlLayout = { state: 'unavailable', reason: 'checking' };
const NO_COPY: ControlLayout = { state: 'no-pushed-copy', orientation: 'landscape', has_controls: true };
const DIFFERS: ControlLayout = { state: 'differs', orientation: 'landscape', has_controls: true };
const MATCHES: ControlLayout = { state: 'matches', orientation: 'landscape', has_controls: true };
const NO_COPY_TEXT = "These Tiles show live values, but their controls aren't set up on this PC.";

const widget = (source: string, field: string, account = '') => ({ type: 'widget' as const, source, field, account });
const at = (id: string, label: string, row: number, col: number, colSpan = 1) => ({ id, label, row, col, rowSpan: 1, colSpan });

const volume: Tile = { ...at('volume', 'Volume', 0, 0, 2), icon: null, render: 'bar', streams: { value: 'number', muted: 'number' }, input: true, action: widget('volume', 'level') };
const forged: Tile = { ...at('forged', 'Volume', 0, 0, 2), icon: null, render: 'bar', streams: { value: 'number', muted: 'number' }, input: true, action: widget('forged', 'level', 'other PC') };
const play: Tile = { ...at('play', 'Play', 1, 0), icon: null, action: { type: 'consumer', usage: 205 } };
const cpu: Tile = { ...at('cpu', 'CPU', 0, 0, 2), icon: null, render: 'graph', streams: { value: 'series' }, action: widget('this_pc', 'cpu') };

const MEDIA: GridPage = { kind: 'grid', id: 'media', title: 'Media', rows: 2, cols: 4, tiles: [volume, play] };
const MORE: GridPage = { kind: 'grid', id: 'more', title: 'More', rows: 2, cols: 4, tiles: [forged] };

const RECOVERED: Layout = { schema: 2, name: 'Upgraded', orientation: 'landscape', pages: [MEDIA, MORE] };
const EARLIER: Layout = { ...RECOVERED, name: 'Earlier', pages: [MEDIA] };
const NO_CONTROLS: Layout = { schema: 2, name: 'Stats', orientation: 'landscape', pages: [{ kind: 'grid', id: 'stats', title: 'Stats', rows: 2, cols: 4, tiles: [cpu, play] }] };

const field = (body: unknown, key: string): unknown => (typeof body === 'object' && body !== null ? new Map(Object.entries(body)).get(key) : undefined);

function text(body: unknown, key: string): string {
  const value = field(body, key);
  if (typeof value !== 'string') throw new Error(`No ${key} in ${JSON.stringify(body)}`);
  return value;
}

type Api = (method: 'GET' | 'PUT' | 'POST', path: string, data?: unknown) => ReturnType<APIRequestContext['fetch']>;

async function companion(request: APIRequestContext): Promise<Api> {
  const minted = await request.post('/__companion/session', { headers: { 'X-TouchDeck-Dev': '1' } });
  expect(minted.status()).toBe(200);
  const token = text(await minted.json(), 'token');
  return (method, path, data) => request.fetch(`/api/v1${path}`, { method, data, headers: { Authorization: `Bearer ${token}` } });
}

async function assessment(api: Api, device = DEVICE): Promise<unknown> {
  return field(await (await api('GET', `/devices/${device}`)).json(), 'control_layout');
}

async function sendFromApi(api: Api, layout: Layout) {
  const pushed = await api('PUT', `/devices/${DEVICE}/layouts/landscape`, layout);
  expect(pushed.status(), await pushed.text()).toBe(202);
  const id = text(await pushed.json(), 'id');
  await expect.poll(async () => field(await (await api('GET', `/jobs/${id}`)).json(), 'state'), { timeout: 30_000 }).toBe('succeeded');
  await expect.poll(() => assessment(api), { timeout: 10_000 }).toMatchObject({ state: 'matches' });
}

async function showWithoutPushedCopy(api: Api, layout: Layout) {
  await sendFromApi(api, layout);
  await rm(PUSHED_COPY);
}

async function showOverPushedCopy(api: Api, layout: Layout, pushed: Layout) {
  await sendFromApi(api, pushed);
  const copy = await readFile(PUSHED_COPY);
  await sendFromApi(api, layout);
  await writeFile(PUSHED_COPY, copy);
}

function controlStates(page: Page): unknown[] {
  const seen: unknown[] = [];
  page.on('websocket', (socket) => {
    if (new URL(socket.url()).pathname !== '/api/v1/events') return;
    socket.on('framereceived', ({ payload }) => {
      const frame: unknown = JSON.parse(String(payload));
      if (field(frame, 'type') !== 'device.updated' || field(frame, 'device') !== DEVICE) return;
      const state = field(field(frame, 'data'), 'control_layout');
      if (JSON.stringify(state) !== JSON.stringify(seen.at(-1))) seen.push(state);
    });
  });
  return seen;
}

const card = (page: Page) => page.locator(`.device-figure:has(.device-hit[aria-label="Edit ${NAME}"])`);
const notice = (page: Page) => page.getByTestId('controls-notice');

async function home(page: Page) {
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await expect(card(page).getByTestId('device-status')).toHaveAccessibleName(/: Connected/);
}

async function shot(page: Page, name: string) {
  if (!shots) return;
  mkdirSync(shots, { recursive: true });
  await page.screenshot({ path: join(shots, `${name}.png`) });
}

test('a Device whose Layout this PC never sent: Home notice, Review Layout, Send, then cleared once the Companion reports matches', async ({ page, request }) => {
  const api = await companion(request);
  const errors = consoleErrors(page);
  const states = controlStates(page);
  await showWithoutPushedCopy(api, RECOVERED);
  await expect.poll(() => assessment(api)).toEqual(NO_COPY);

  await home(page);
  const desk = card(page);
  await expect(desk.getByTestId('controls-notice')).toHaveAttribute('data-state', 'no-pushed-copy');
  await expect(desk.getByTestId('controls-notice')).toContainText('Controls need Send to Device');
  await shot(page, 'companion-home-no-copy');

  await desk.getByRole('button', { name: `Review Layout for ${NAME}` }).click();
  await expect(page.getByTestId('editor')).toHaveAttribute('data-device-id', DEVICE);
  await waitEvent(page, 'pulled', 30_000);
  await expect(notice(page)).toHaveAttribute('data-state', 'no-pushed-copy');
  await expect(page.getByTestId('controls-notice-body')).toHaveText(NO_COPY_TEXT);
  await expect(notice(page)).toContainText(`Sending replaces the Layout on ${NAME} and lets its Tiles control the Integrations shown here.`);

  const rows = page.getByTestId('control-binding');
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0).getByTestId('control-integration')).toHaveText('Volume');
  await expect(rows.nth(0).getByTestId('control-account')).toHaveText('Default account');
  await expect(rows.nth(0).getByTestId('control-parts')).toHaveText('Drag to change · Mute');
  await expect(rows.nth(1)).toHaveAttribute('data-page', 'more');
  await expect(rows.nth(1).getByTestId('control-integration')).toHaveText('Unknown Integration “forged”');
  await expect(rows.nth(1).getByTestId('control-account')).toHaveText('Unknown account “other PC”');
  await shot(page, 'companion-review');

  await rows.nth(1).click();
  await page.getByTestId('tile-clear').click();
  await expect(rows).toHaveCount(1);

  states.length = 0;
  await page.getByTestId('push').click();
  await expect(await waitEvent(page, 'pushed', 30_000)).toHaveText(`Sent to ${NAME}`);
  await expect(notice(page)).toHaveCount(0, { timeout: 10_000 });
  await expect(page.getByTestId('control-review')).toHaveCount(0);
  expect(states).toEqual([CHECKING, MATCHES]);
  expect(await assessment(api)).toEqual(MATCHES);

  await page.getByRole('button', { name: 'Home', exact: true }).click();
  await expect(card(page).getByTestId('controls-notice')).toHaveCount(0);
  expect(errors.filter((line) => !line.startsWith('Failed to load resource'))).toEqual([]);
});

test('Review Layout goes through the load flow and keeps an unsent draft until the user replaces it', async ({ page, request }) => {
  const api = await companion(request);
  await showWithoutPushedCopy(api, RECOVERED);
  await expect.poll(() => assessment(api)).toEqual(NO_COPY);

  await home(page);
  await card(page).locator('.device-hit').click();
  await page.getByTestId('start-new').click();
  await expect(page.getByTestId('save-state')).toHaveAttribute('data-state', 'unsent');
  await page.getByTestId('controls-review').click();
  await expect(page.getByTestId('replace-dialog')).toBeVisible();
  await page.getByTestId('keep-mine').click();
  await expect(page.getByTestId('page-tab')).toHaveCount(1);
  await expect(page.getByTestId('control-review')).toContainText('No Tile on this Layout controls an Integration.');
  await expect(notice(page)).toHaveAttribute('data-state', 'no-pushed-copy');

  await page.getByRole('button', { name: 'Home', exact: true }).click();
  await card(page).getByTestId('controls-review').click();
  await page.getByTestId('replace').click();
  await expect(page.getByTestId('page-tab')).toHaveCount(2);
  await expect(page.getByTestId('control-binding')).toHaveCount(2);
});

test('a Layout that differs from the one this PC sent says so', async ({ page, request }) => {
  const api = await companion(request);
  await showOverPushedCopy(api, RECOVERED, EARLIER);
  await expect.poll(() => assessment(api)).toEqual(DIFFERS);

  await home(page);
  await expect(card(page).getByTestId('controls-notice')).toHaveAttribute('data-state', 'differs');
  await card(page).getByTestId('controls-review').click();
  await waitEvent(page, 'pulled', 30_000);
  await expect(page.getByTestId('controls-notice-body')).toHaveText(`The Layout on ${NAME} differs from the one this PC sent.`);
  await shot(page, 'companion-differs');
});

test('the editor on the other orientation names the one that needs review and switches to it', async ({ page, request }) => {
  const api = await companion(request);
  await showWithoutPushedCopy(api, RECOVERED);
  await expect.poll(() => assessment(api)).toEqual(NO_COPY);

  await home(page);
  await card(page).getByTestId('controls-review').click();
  await waitEvent(page, 'pulled', 30_000);
  await page.getByTestId('orientation-portrait').click();
  await expect(notice(page)).toHaveAttribute('data-state', 'other-orientation');
  await expect(page.getByTestId('controls-notice-body')).toHaveText(`${NAME} shows its landscape Layout. Switch to Landscape to review it.`);
  await expect(page.getByTestId('control-review')).toHaveCount(0);
  await page.getByTestId('controls-switch').click();
  await expect(page.getByTestId('orientation-landscape')).toHaveAttribute('aria-pressed', 'true');
  await expect(notice(page)).toHaveAttribute('data-state', 'no-pushed-copy');
  await expect(page.getByTestId('control-binding')).toHaveCount(2);
});

test('a matching Layout, a Device without live values, a Layout without controls and an offline Device show nothing new', async ({ page, request }) => {
  const api = await companion(request);
  await sendFromApi(api, RECOVERED);
  await expect.poll(() => assessment(api)).toEqual(MATCHES);
  await home(page);
  await expect(card(page).getByTestId('controls-notice')).toHaveCount(0);

  expect((await api('POST', '/test/ports', { plug: { id: 'mock00c1', kind: 'device-v1' } })).status()).toBe(204);
  await expect(page.getByRole('list', { name: 'Devices' }).getByRole('listitem')).toHaveCount(4);
  await expect.poll(() => assessment(api, 'mock00c1'), { timeout: 10_000 }).toEqual({ state: 'unavailable', reason: 'unsupported' });
  await expect(page.getByTestId('controls-notice')).toHaveCount(0);
  expect((await api('POST', '/test/ports', { unplug: 'mock00c1' })).status()).toBe(204);

  await showWithoutPushedCopy(api, NO_CONTROLS);
  await expect.poll(() => assessment(api)).toEqual({ ...NO_COPY, has_controls: false });
  await expect(card(page).getByTestId('device-status')).toHaveAccessibleName(/: Connected/);
  await expect(card(page).getByTestId('controls-notice')).toHaveCount(0);

  await showWithoutPushedCopy(api, RECOVERED);
  await expect(card(page).getByTestId('controls-notice')).toHaveAttribute('data-state', 'no-pushed-copy');
  expect((await api('POST', '/test/ports', { unplug: DEVICE })).status()).toBe(204);
  await expect(card(page).locator('.device-meta .status')).toHaveText('Not connected');
  await expect.poll(() => assessment(api)).toEqual({ state: 'unavailable', reason: 'offline' });
  await expect(card(page).getByTestId('controls-notice')).toHaveCount(0);
  await card(page).locator('.device-hit').click();
  await expect(page.getByTestId('disconnected')).toBeVisible();
  await expect(notice(page)).toHaveCount(0);

  expect((await api('POST', '/test/ports', { plug: { id: DEVICE, kind: 'device' } })).status()).toBe(204);
  await expect.poll(() => assessment(api), { timeout: 10_000 }).toEqual(NO_COPY);
});
