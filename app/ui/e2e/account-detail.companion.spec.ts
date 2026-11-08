import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { createServer } from 'node:http';
import { join, resolve } from 'node:path';
import { expect, test } from '@playwright/test';

test.use({ trace: 'off', screenshot: 'off', video: 'off' });

test('one mock Device and a local hub expose account detail without leaking credentials', async ({ request }) => {
  const paths: string[] = [];
  const hub = createServer((req, res) => {
    paths.push(`${req.method} ${req.url}`);
    res.setHeader('Content-Type', 'application/json');
    if (req.method === 'GET' && req.url === '/v0/management/auth-files') {
      res.end(JSON.stringify({ files: [
        { provider: 'codex', label: 'Work', status: 'active', auth_index: 'AX17-private-BX29', success: 128, failed: 2 },
        { provider: 'claude', label: 'Personal', status: 'active', auth_index: 'EX59-private-FX61' },
      ] }));
    } else if (req.method === 'POST' && req.url === '/v0/management/api-call') {
      res.end(JSON.stringify({ status_code: 403, body: 'GX73-private-HX89' }));
    } else {
      res.statusCode = 404;
      res.end('{}');
    }
  });
  hub.listen(0, '127.0.0.1');
  await once(hub, 'listening');
  const address = hub.address();
  if (!address || typeof address === 'string') throw new Error('Stub address missing');
  const data = await mkdtemp(resolve('../target/detail-e2e-'));
  const child = spawn(resolve('../target/debug', process.platform === 'win32' ? 'td-companion.exe' : 'td-companion'), ['--mock-devices', '1'], {
    env: { ...process.env, TOUCHDECK_APP_DATA_DIR: data }, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
  });
  let logs = '';
  child.stdout.setEncoding('utf8').on('data', (text: string) => { logs += text; });
  child.stderr.setEncoding('utf8').on('data', (text: string) => { logs += text; });
  try {
    let origin = '';
    let token = '';
    await expect.poll(async () => {
      try {
        const runtime: unknown = JSON.parse(await readFile(join(data, 'runtime/companion.json'), 'utf8'));
        if (typeof runtime !== 'object' || runtime === null || !('port' in runtime) || typeof runtime.port !== 'number' || !('token' in runtime) || typeof runtime.token !== 'string') return false;
        origin = `http://127.0.0.1:${runtime.port}`;
        const session: unknown = await (await request.post(`${origin}/api/v1/auth/sessions`, { headers: { Authorization: `Bearer ${runtime.token}` }, data: { kind: 'window' } })).json();
        if (typeof session !== 'object' || session === null || !('token' in session) || typeof session.token !== 'string') return false;
        token = session.token;
        return true;
      } catch { return false; }
    }).toBe(true);
    const headers = { Authorization: `Bearer ${token}` };
    const url = `${origin}/api/v1/integrations/cliproxyapi`;
    expect((await request.put(`${url}/config`, { headers, data: { enabled: true, url: `http://127.0.0.1:${address.port}`, interval_s: 60 } })).ok()).toBe(true);
    expect((await request.put(`${url}/secrets/management_key`, { headers, data: { value: 'IX97-private-JX03' } })).ok()).toBe(true);
    expect((await request.post(`${url}/refresh`, { headers })).ok()).toBe(true);
    let text = '';
    await expect.poll(async () => {
      text = await (await request.get(url, { headers })).text();
      const polled: unknown = JSON.parse(text);
      return typeof polled === 'object' && polled !== null && 'detail' in polled && polled.detail !== null;
    }, { timeout: 30_000 }).toBe(true);
    const view: unknown = JSON.parse(text);
    expect(view).toHaveProperty('detail.accounts.0.account', 'Work');
    expect(view).toHaveProperty('detail.accounts.1.error', "The service refused this account's sign-in (HTTP 403). Sign in to it again in CLIProxyAPI.");
    await expect.poll(async () => (await request.get(`${origin}/api/v1/devices`, { headers })).json()).toEqual([expect.objectContaining({ id: 'mock0001' })]);
    expect(paths).toEqual(['GET /v0/management/auth-files', 'POST /v0/management/api-call', 'POST /v0/management/api-call']);
    for (const secret of ['AX17-private-BX29', 'EX59-private-FX61', 'GX73-private-HX89', 'IX97-private-JX03', token]) {
      for (const part of [secret, secret.slice(0, 4), secret.slice(-4)]) expect(text + logs).not.toContain(part);
    }
  } finally {
    if (child.exitCode === null) {
      const exited = once(child, 'exit');
      child.kill();
      await exited;
    }
    hub.closeAllConnections();
    await new Promise<void>((done, reject) => hub.close(error => error ? reject(error) : done()));
    await rm(data, { recursive: true, force: true });
  }
});
