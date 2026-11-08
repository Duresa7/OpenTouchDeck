import { test, expect, _electron as electron, type ElectronApplication } from '@playwright/test';
import { spawn, type ChildProcess } from 'node:child_process';
import { createWriteStream, existsSync } from 'node:fs';
import { mkdir, mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import type {} from '../../ui/src/lib/companion/session';

// Build with `cargo build -p td-companion --features test-hooks` in app/, or point TOUCHDECK_COMPANION_BIN at a build.
const binary = process.env.TOUCHDECK_COMPANION_BIN ||
  resolve('../target/debug/td-companion' + (process.platform === 'win32' ? '.exe' : ''));
const evidence = process.env.TOUCHDECK_EVIDENCE_DIR || resolve('../../.verify/72');

/** Keeps a process's output with the spec's results, so a CI failure shows what the Companion or Electron said. */
function record(child: ChildProcess, name: string) {
  const log = createWriteStream(test.info().outputPath(name));
  child.stdout?.on('data', chunk => log.write(chunk));
  child.stderr?.on('data', chunk => log.write(chunk));
  child.once('close', () => log.end());
}

/** Launches the Electron window and records its main-process output. */
async function launch(env: NodeJS.ProcessEnv): Promise<ElectronApplication> {
  const app = await electron.launch({ args: [resolve('.')], env });
  record(app.process(), 'electron.log');
  return app;
}

/** Runs `td-companion --mock-devices 1` in a scratch data dir for the length of `body`. */
async function withCompanion(body: (env: NodeJS.ProcessEnv, origin: string) => Promise<void>) {
  expect(existsSync(binary), `Companion binary missing at ${binary}`).toBe(true);
  const dir = await mkdtemp(join(tmpdir(), 'touchdeck-companion-'));
  const env = { ...process.env, TOUCHDECK_APP_DATA_DIR: dir, TOUCHDECK_COMPANION_BIN: binary };
  delete env.ELECTRON_RUN_AS_NODE;
  const companion = spawn(binary, ['--mock-devices', '1'], { env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  record(companion, 'companion.log');
  const runtimeFile = join(dir, 'runtime', 'companion.json');
  try {
    await expect.poll(() => existsSync(runtimeFile), { timeout: 10_000 }).toBe(true);
    const { port } = JSON.parse(await readFile(runtimeFile, 'utf8'));
    await body(env, `http://127.0.0.1:${port}`);
  } finally {
    companion.kill();
    await rm(dir, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
}

test('window loads from the real Companion and its session lists the mock Device', async () => {
  await withCompanion(async (env, origin) => {
    const app = await launch(env);
    try {
      const page = await app.firstWindow();
      await page.waitForURL(origin + '/');
      expect(await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].getTitle())).toBe('Open TouchDeck');
      const result = await page.evaluate(async () => {
        const token = await window.openTouchDeck?.session();
        const response = await fetch('/api/v1/devices', { headers: { Authorization: `Bearer ${token}` } });
        return { token, status: response.status, body: await response.json() };
      });
      expect(result.status).toBe(200);
      const devices = Array.isArray(result.body) ? result.body : result.body.devices;
      expect(devices.map((d: { id: string }) => d.id)).toEqual([expect.stringContaining('mock0001')]);
      await mkdir(evidence, { recursive: true });
      await page.screenshot({ path: resolve(evidence, 'electron-companion.png') });
      await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].close());
      // Closing revoked the window session on the Companion.
      await expect.poll(async () => (await fetch(`${origin}/api/v1/devices`,
        { headers: { Authorization: `Bearer ${result.token}` } })).status).toBe(401);
    } finally { await app.close(); }
  });
});

test('in development the window shows Home from the UI dev server with the mock Device', async () => {
  await withCompanion(async env => {
    const ui = 'http://127.0.0.1:5176/';
    let vite: ChildProcess | undefined;
    try {
      vite = spawn(process.execPath, [resolve('../ui/node_modules/vite/bin/vite.js'), '--port', '5176'],
        { cwd: resolve('../ui'), env, windowsHide: true, stdio: 'ignore' });
      await expect.poll(async () => { try { return (await fetch(ui)).ok; } catch { return false; } },
        { timeout: 30_000 }).toBe(true);
      const app = await launch({ ...env, TOUCHDECK_UI_DEV_URL: ui });
      try {
        const page = await app.firstWindow();
        await page.waitForURL(ui);
        const cards = page.getByRole('list', { name: 'Devices' }).getByRole('listitem');
        await expect(cards).toHaveCount(1, { timeout: 15_000 });
        await expect(cards.first()).toContainText('Connected');
        // The dev page signs in through the dev server; Electron gives the window session only to the Companion origin.
        expect(await page.evaluate(() => window.openTouchDeck?.session())).toBe(null);
        await mkdir(evidence, { recursive: true });
        await page.screenshot({ path: resolve(evidence, 'electron-home.png') });
      } finally { await app.close(); }
    } finally { vite?.kill(); }
  });
});

test('reading a session preserves in-flight requests and stale renewals share one replacement', async () => {
  await withCompanion(async env => {
    const app = await launch(env);
    try {
      const page = await app.firstWindow();
      await page.waitForLoadState();
      const result = await page.evaluate(async () => {
        const bridge = window.openTouchDeck;
        if (!bridge) throw new Error('Window bridge missing');
        const first = await bridge.session();
        if (!first) throw new Error('Window session missing');
        const second = await bridge.session();
        const status = async (token: string) => (await fetch('/api/v1/devices', {
          headers: { Authorization: `Bearer ${token}` },
        })).status;
        const inFlightStatus = await status(first);
        const renewed = await Promise.all([bridge.session(first), bridge.session(first)]);
        const late = await bridge.session(first);
        const read = await bridge.session();
        return {
          sameRead: first === second, inFlightStatus,
          replaced: renewed[0] !== first,
          shared: renewed.every(token => token === late) && late === read,
          oldStatus: await status(first),
          newStatus: late ? await status(late) : null,
        };
      });
      expect(result).toEqual({ sameRead: true, inFlightStatus: 200, replaced: true,
        shared: true, oldStatus: 401, newStatus: 200 });
    } finally { await app.close(); }
  });
});

test('a saved Brandfetch client ID opens the window to Brandfetch after a reload, and forgetting it closes it', async () => {
  // A made-up client ID; every Brandfetch host is answered here, so nothing reaches Brandfetch.
  const hits = [{ name: 'Example Music', domain: 'music.example', claimed: true, brandId: 'id_fixture_music',
    icon: 'https://cdn.brandfetch.io/id_fixture_music/icon.png?c=test-client-id' }];
  const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==', 'base64');
  await withCompanion(async (env, origin) => {
    const app = await launch(env);
    try {
      const searches: { url: string; referer: string | null }[] = [];
      await app.context().route('https://api.brandfetch.io/**', async route => {
        searches.push({ url: route.request().url(), referer: await route.request().headerValue('referer') });
        await route.fulfill({ status: 200, headers: { 'access-control-allow-origin': '*' }, contentType: 'application/json', body: JSON.stringify(hits) });
      });
      await app.context().route(/^https:\/\/(cdn|asset)\.brandfetch\.io\//, route =>
        route.fulfill({ status: 200, contentType: 'image/png', body: png }));
      const page = await app.firstWindow();
      await page.waitForURL(origin + '/');
      await expect(page.getByRole('list', { name: 'Devices' })).toBeVisible({ timeout: 15_000 });
      // Whether the page's content policy lets it call the Brand API search.
      const reaches = () => page.evaluate(() => fetch('https://api.brandfetch.io/v2/search/probe?c=test-client-id',
        { credentials: 'omit' }).then(() => true, () => false));
      const view = page.locator('[data-testid="integration-detail"][data-id="brandfetch"]');
      const openBrandfetch = async () => {
        await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Integrations' }).click();
        await page.locator('[data-testid="integration"][data-id="brandfetch"]').getByTestId('integration-settings').click();
        await expect(view).toBeVisible();
      };
      const openPicker = async () => {
        await page.getByRole('button', { name: 'Layout', exact: true }).click();
        const start = page.getByTestId('start-starter');
        await expect(start.or(page.getByTestId('page-tab').first())).toBeVisible({ timeout: 15_000 });
        if (await start.isVisible()) await start.click();
        await page.getByTestId('page-tab').filter({ hasText: 'Shortcuts' }).click();
        await page.locator('[data-testid="tile"][data-id="copy"]').click();
        await page.getByTestId('icon-change').click();
        await expect(page.getByTestId('icon-picker').getByTestId('icon-grid')).toBeVisible();
      };

      expect(await reaches()).toBe(false);
      expect(searches).toEqual([]);

      await openBrandfetch();
      await view.getByTestId('brandfetch-client-id-input').fill('test-client-id');
      let reloaded = page.waitForEvent('load');
      await view.getByTestId('brandfetch-client-id-save').click();
      await reloaded;
      // The reload comes back to the same Integrations page.
      await expect(view.getByTestId('brandfetch-client-id-state')).toHaveText('test-client-id');
      await openPicker();
      await page.getByTestId('icon-search').fill('example');
      const hit = page.getByTestId('icon-picker').locator('[data-testid="icon-option"][data-key="brand:id_fixture_music"]');
      await expect(hit).toBeVisible();
      expect(searches.map(s => new URL(s.url).pathname)).toEqual(['/v2/search/example']);
      // The page itself sends no Referer; the search sends only the Companion's origin.
      expect(searches[0].referer).toBe(origin + '/');
      await expect.poll(() => hit.locator('img').evaluate((img: HTMLImageElement) => img.complete && img.naturalWidth > 0)).toBe(true);
      await mkdir(evidence, { recursive: true });
      await page.screenshot({ path: resolve(evidence, 'electron-brandfetch-search.png') });
      await page.keyboard.press('Escape');

      await openBrandfetch();
      await view.getByTestId('brandfetch-client-id-forget').click();
      reloaded = page.waitForEvent('load');
      await page.getByTestId('forget-confirm').click();
      await reloaded;
      await expect(view.getByTestId('brandfetch-client-id-field')).toHaveAttribute('data-set', 'false');
      expect(await reaches()).toBe(false);
      await openPicker();
      await page.getByTestId('icon-search').fill('example');
      await expect(page.getByTestId('icon-picker').locator('.icon-credits')).not.toContainText('Brandfetch');
      await expect(page.getByTestId('icon-picker').getByTestId('icon-section-brandfetch')).toHaveCount(0);
      expect(searches).toHaveLength(1);
    } finally { await app.close(); }
  });
});
