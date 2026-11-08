import { expect, test, type APIRequestContext, type Page } from '@playwright/test';
import { start, type FixtureServer } from './fixtures/cliproxy-server.mjs';

test.describe.configure({ mode: 'serial' });

const KEY = 'companion-e2e-secret-4b2e19';
const FORBIDDEN = /usage-queue|reset-quota|credit-consume/i;

let hub: FixtureServer;

test.beforeAll(async () => {
  hub = await start({ key: KEY });
});

test.afterAll(async () => {
  await hub.stop();
});

async function companion(request: APIRequestContext) {
  const minted = await request.post('/__companion/session', { headers: { 'X-TouchDeck-Dev': '1' } });
  expect(minted.status()).toBe(200);
  const session: unknown = await minted.json();
  if (typeof session !== 'object' || session === null || !('token' in session) || typeof session.token !== 'string') throw new Error('Missing session');
  const { token } = session;
  return async (path: string) => {
    const response = await request.get(`/api/v1${path}`, { headers: { Authorization: `Bearer ${token}` } });
    expect(response.status()).toBe(200);
    return response.text();
  };
}

async function openCliProxy(page: Page) {
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.getByRole('button', { name: 'Integrations', exact: true }).click();
  await page.locator('[data-testid="integration"][data-id="cliproxyapi"]').getByTestId('integration-settings').click();
  await expect(page.getByTestId('integration-detail')).toHaveAttribute('data-id', 'cliproxyapi');
}

const field = (page: Page, name: string) => page.locator(`[data-testid="config-field"][data-field="${name}"]`);
const outcome = (page: Page, at = 'settings') => page.locator(`[data-testid="outcome"][data-at="${at}"]`);

test('CLIProxyAPI is set up against the local hub, and its key shows only as Set', async ({ page, request }) => {
  const api = await companion(request);
  await openCliProxy(page);
  await expect(page.getByTestId('secret-field')).toHaveAttribute('data-set', 'false');

  await page.getByTestId('detail-switch').click();
  await expect(page.getByTestId('detail-switch')).toHaveAttribute('aria-busy', 'true');
  await field(page, 'url').fill(hub.url);
  await page.getByTestId('config-save').click();
  await expect(outcome(page)).toHaveAttribute('data-event', 'saved');

  await page.getByTestId('secret-input').fill(KEY);
  await page.getByTestId('secret-save').click();
  await expect(page.getByTestId('secret-state')).toHaveText('Set');
  await expect(page.getByTestId('secret-input')).toHaveCount(0);
  await expect(page.getByTestId('detail-switch')).not.toHaveAttribute('aria-busy', 'true');
  await expect(page.getByTestId('detail-switch')).toHaveAttribute('aria-checked', 'true');
  expect(await page.content()).not.toContain(KEY);

  const view = await api('/integrations/cliproxyapi');
  expect(view).not.toContain(KEY);
  expect(JSON.parse(view)).toMatchObject({ summary: { enabled: true }, config: { url: hub.url }, secrets: { management_key: { set: true } } });
  expect(await api('/integrations')).not.toContain(KEY);

  await openCliProxy(page);
  await expect(page.getByTestId('secret-state')).toHaveText('Set');
  await expect(page.getByTestId('secret-input')).toHaveCount(0);

  await page.getByTestId('refresh').click();
  await expect(outcome(page, 'status')).toHaveAttribute('data-event', 'refreshed');
  await expect(page.getByTestId('usage-account')).toHaveCount(4);
  await expect(page.getByTestId('usage-five-hour').first()).toContainText('42%');
});

test('moving CLIProxyAPI to another origin makes the Companion forget the key, and the view says so', async ({ page, request }) => {
  const api = await companion(request);
  await openCliProxy(page);
  await expect(page.getByTestId('secret-field')).toHaveAttribute('data-set', 'true');
  await field(page, 'url').fill(hub.url.replace('127.0.0.1', 'localhost'));
  await page.getByTestId('config-save').click();
  await expect(outcome(page)).toHaveAttribute('data-event', 'secret-forgotten');
  await expect(outcome(page)).toHaveText('Saved. The address changed, so the management key was forgotten. Enter it again.');
  await expect(page.getByTestId('secret-field')).toHaveAttribute('data-set', 'false');
  expect(JSON.parse(await api('/integrations/cliproxyapi'))).toMatchObject({ secrets: { management_key: { set: false } } });

  await field(page, 'url').fill(hub.url);
  await page.getByTestId('config-save').click();
  await expect(outcome(page)).toHaveAttribute('data-event', 'saved');
  expect(JSON.parse(await api('/integrations/cliproxyapi'))).toMatchObject({ summary: { enabled: true } });
});

test('the hub never sees a usage-queue, reset-quota or credit-consume call', () => {
  expect(hub.requests.filter((r) => FORBIDDEN.test(`${r.path ?? ''} ${r.body?.url ?? ''}`))).toEqual([]);
  expect(hub.requests.every((r) => !r.path?.includes(KEY) && !JSON.stringify(r.body ?? {}).includes(KEY))).toBe(true);
});
