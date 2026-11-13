import { readFileSync, writeFileSync, mkdirSync, existsSync, unlinkSync, openSync, closeSync, writeSync } from 'node:fs';
import { join, dirname, isAbsolute } from 'node:path';
import { createRequire } from 'node:module';
import { spawn, spawnSync } from 'node:child_process';
import { createHmac, randomBytes, timingSafeEqual } from 'node:crypto';
import { setTimeout as delay } from 'node:timers/promises';
import net from 'node:net';
const [root, command, ...args] = process.argv.slice(2);
const app = join(root, 'app'), ui = join(app, 'ui'), target = join(app, 'target');
// One lock per machine: it lives in the main checkout's .verify/, shared by every worktree.
function mainCheckout() {
  const common = spawnSync('git', ['-C', root, 'rev-parse', '--path-format=absolute', '--git-common-dir'], { encoding: 'utf8' }).stdout?.trim();
  return common ? dirname(common) : root;
}
const hardwareLock = process.env.TOUCHDECK_HARDWARE_LOCK || join(mainCheckout(), '.verify/hardware.lock');
const binary = join(target, 'debug/td-companion.exe'), stateFile = join(target, 'tdapp-state.json');
const require = createRequire(join(ui, 'package.json'));
function findPnpm() {
  const paths = spawnSync('where.exe', ['pnpm'], { encoding: 'utf8' }).stdout?.trim().split(/\r?\n/) ?? [];
  for (const path of paths) {
    if (path.endsWith('.exe')) return path;
    const native = join(dirname(path), 'node_modules/pnpm/pnpm.exe');
    if (existsSync(native)) return native;
  }
  return undefined;
}
const pnpmBinary = findPnpm();
const opt = (name, fallback) => { const i = args.indexOf(name); if (i < 0) return fallback; if (!args[i + 1]) throw Error(`${name} needs a value`); return args[i + 1]; };
const json = path => JSON.parse(readFileSync(path, 'utf8').replace(/^\uFEFF/, ''));
const save = (path, value) => { mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, JSON.stringify(value) + '\n'); };
function run(program, argv, cwd = app, env = process.env) {
  if (program === 'pnpm' && !pnpmBinary) throw Error('No native pnpm executable; report to coordinator without reinstalling');
  const result = spawnSync(program === 'pnpm' ? pnpmBinary : program, argv, { cwd, env, stdio: ['ignore', process.stderr.fd, process.stderr.fd] });
  if (result.error) throw result.error;
  return result.status ?? 2;
}
function checked(program, argv, env) { const code = run(program, argv, app, env); if (code) throw Error(`${program} ${argv.join(' ')} exited ${code}`); }
const head = () => { const r = spawnSync('git', ['-C', root, 'rev-parse', 'HEAD'], { encoding: 'utf8' }); if (r.status) throw Error('Cannot read HEAD'); return r.stdout.trim(); };
const state = () => existsSync(stateFile) ? json(stateFile) : null;
function identity(pid) {
  const script = `$p = Get-Process -Id ${Number(pid)} -ErrorAction SilentlyContinue; if ($p) { @{path=$p.Path; ticks=$p.StartTime.ToUniversalTime().Ticks.ToString()} | ConvertTo-Json -Compress }`;
  const result = spawnSync('pwsh', ['-NoProfile', '-Command', script], { encoding: 'utf8' });
  return result.stdout.trim() ? JSON.parse(result.stdout) : null;
}
function owned(s) { const p = s?.root === root ? identity(s.pid) : null; return p && p.path.toLowerCase() === s.binary.toLowerCase() && p.ticks === s.ticks; }
async function freePort(port = 0) { const server = net.createServer(); await new Promise((resolve, reject) => server.once('error', reject).listen(port, '127.0.0.1', resolve)); const chosen = server.address().port; await new Promise(resolve => server.close(resolve)); return chosen; }
async function session(data) {
  const runtime = json(join(data, 'runtime/companion.json')), origin = `http://127.0.0.1:${runtime.port}`;
  async function request(path, method = 'GET', body, token) {
    if (!path.startsWith('/') || path.startsWith('//')) throw Error('Use an API path');
    const r = await fetch(`${origin}/api/v1${path}`, { method, redirect: 'error', signal: AbortSignal.timeout(20000), headers: { 'Content-Type': 'application/json', ...(token ? { Authorization: `Bearer ${token}` } : {}) }, body: body || undefined });
    const text = await r.text(); return { ok: r.ok, status: r.status, body: text ? JSON.parse(text) : null };
  }
  const nonce = randomBytes(32), proof = await request('/auth/proof', 'POST', JSON.stringify({ nonce: nonce.toString('base64url') }));
  const expected = createHmac('sha256', runtime.token).update('touchdeck-companion-proof\0').update(nonce).digest();
  const actual = Buffer.from(proof.body?.proof ?? '', 'base64url');
  if (actual.length !== expected.length || !timingSafeEqual(actual, expected)) throw Error('Companion proof failed');
  const minted = await request('/auth/sessions', 'POST', JSON.stringify({ kind: 'window', label: 'Verification' }), runtime.token);
  if (!minted.ok) throw Error('Cannot mint verification session');
  const token = minted.body.token;
  return { origin, token, request: (path, method, body) => request(path, method, body, token), close: () => request('/auth/sessions/self', 'DELETE', undefined, token).catch(() => {}) };
}
async function stop() {
  const s = state(), stopped = [];
  if (owned(s)) {
    try { const client = await session(s.data); await client.request('/system/shutdown', 'POST'); await client.close(); } catch {}
    for (let i = 0; i < 25 && owned(s); i++) await delay(200);
    if (owned(s)) process.kill(s.pid);
    for (let i = 0; i < 25 && owned(s); i++) await delay(200);
    stopped.push(s.pid);
  }
  const remaining = Number(Boolean(owned(s)));
  if (!remaining && existsSync(stateFile)) unlinkSync(stateFile);
  return { ok: !remaining, stopped, remaining };
}
async function start(mock, port, data, deviceConsole = false, log = join(data, 'companion.log')) {
  if (!isAbsolute(data)) throw Error('--data must be absolute');
  if (!isAbsolute(log)) throw Error('--log must be absolute');
  if (owned(state())) throw Error('This checkout already has a recorded Companion; stop first');
  if (!existsSync(binary)) throw Error('Missing debug Companion; run build');
  if (!mock && !existsSync(hardwareLock)) throw Error(`Real Device access requires ${hardwareLock}`);
  if (mock && deviceConsole) throw Error('--device-console is for the real Companion; mock Devices already take devcmd');
  if (!Number.isInteger(port) || port < 49152 || port > 65535) throw Error('Companion port must be 49152..65535');
  await freePort(port); save(join(data, 'companion-settings.json'), { port });
  const argv = mock ? ['--mock-devices', '3'] : deviceConsole ? ['--test-real-device-console'] : [];
  mkdirSync(dirname(log), { recursive: true });
  const out = openSync(log, 'a');
  writeSync(out, `--- tdapp start ${new Date().toISOString()} ${[binary, ...argv].join(' ')}
`);
  let child;
  try {
    child = spawn(binary, argv, { cwd: app, env: { ...process.env, TOUCHDECK_APP_DATA_DIR: data }, windowsHide: true, detached: true, stdio: ['ignore', out, out] });
    await new Promise((resolve, reject) => { child.once('spawn', resolve); child.once('error', reject); });
  } finally { closeSync(out); }
  const id = identity(child.pid), s = { root, pid: child.pid, binary, ticks: id?.ticks, port, mock, data, deviceConsole, log };
  save(stateFile, s); child.unref();
  try {
    for (let i = 0; i < 150; i++) {
      if (!owned(s)) throw Error('Companion exited before ready');
      try {
        if (json(join(data, 'runtime/companion.json')).port !== port) throw Error('Companion selected another port');
        const client = await session(data);
        try { const info = await client.request('/system'); if (info.ok) return { ok: true, pid: s.pid, port, mock, deviceConsole, data, log, build: info.body.build }; } finally { await client.close(); }
      } catch { await delay(200); }
    }
    throw Error('Companion did not answer within 30 seconds');
  } catch (error) { await stop(); throw error; }
}
async function e2e() {
  const spec = args[0]?.replace(/\.spec\.ts$/, '');
  if (!/^[a-z][a-z0-9.-]*$/.test(spec) || !existsSync(join(ui, 'e2e', `${spec}.spec.ts`))) throw Error('Name one spec from app/ui/e2e');
  const real = args.includes('--companion');
  if (real !== (spec === 'companion' || spec.endsWith('.companion'))) throw Error('Use --companion for Companion specs only');
  const out = opt('--out', join(target, 'tdapp-e2e', spec));
  if (!isAbsolute(out)) throw Error('--out must be absolute');
  const port = Number(opt('--port', real ? '50844' : '5184'));
  if (!Number.isInteger(port) || port < (real ? 49152 : 1024) || port > 65534) throw Error('Invalid UI port');
  await freePort(port); if (real) await freePort(port + 1);
  const data = join(out, 'data'); mkdirSync(data, { recursive: true });
  if (real) save(join(data, 'companion-settings.json'), { port: port + 1 });
  const config = join(ui, `.tdapp-${randomBytes(8).toString('hex')}.config.mjs`);
  const url = `http://127.0.0.1:${port}/`, env = { TOUCHDECK_APP_DATA_DIR: data, TD_SHOTS: out };
  const servers = [];
  if (real) servers.push({ command: `"${binary}" --mock-devices 3`, cwd: ui, env, url: `http://127.0.0.1:${port + 1}/api/v1/system/ping`, reuseExistingServer: false, timeout: 30000 });
  servers.push({ command: `pnpm ${real ? 'dev' : 'dev:demo'} --port ${port}`, cwd: ui, env, url, reuseExistingServer: false, timeout: 60000 });
  const q = JSON.stringify;
  writeFileSync(config, `import base from './playwright.config.ts';\nexport default {...base,testMatch:${q(spec + '.spec.ts')},testIgnore:[],workers:1,reporter:[['list'],['json',{outputFile:${q(join(out, 'report.json'))}}]],outputDir:${q(join(out, 'results'))},use:{...base.use,baseURL:${q(url)},screenshot:'on',trace:'retain-on-failure'},webServer:${q(servers)}};\n`);
  try { const code = run('pnpm', ['exec', 'playwright', 'test', '--config', config, `${spec}.spec.ts`], ui); process.exitCode = code; return { ok: !code, exit: code, spec, companion: real, report: join(out, 'report.json'), out }; }
  finally { unlinkSync(config); }
}
async function main() {
  mkdirSync(target, { recursive: true });
  if (command === 'build') {
    const version = spawnSync(pnpmBinary, ['--version'], { encoding: 'utf8' });
    if (version.status || !version.stdout.trim()) throw Error('Broken pnpm; report to coordinator without reinstalling');
    const bundle = opt('--firmware-bundle');
    if (bundle !== undefined && (!isAbsolute(bundle) || !existsSync(bundle))) throw Error('--firmware-bundle needs an absolute path to an existing .tdfw');
    const env = bundle === undefined ? process.env : { ...process.env, TOUCHDECK_FIRMWARE_BUNDLE: bundle };
    checked('pnpm', ['install', '--frozen-lockfile']); checked('pnpm', ['build:ui']);
    checked('cargo', ['build', '-p', 'td-companion', '--features', 'test-hooks'], env); checked('cargo', ['build', '-p', 'td-link']);
    return { ok: true, companion: binary, td_link: join(target, 'debug/td-link.exe'), head_sha: head(), test_hooks: true, firmware_bundle: env.TOUCHDECK_FIRMWARE_BUNDLE ?? null };
  }
  if (command === 'start') { const data = opt('--data', join(target, 'tdapp-data')); return start(args.includes('--mock'), Number(opt('--port', '50744')), data, args.includes('--device-console'), opt('--log', join(data, 'companion.log'))); }
  if (command === 'stop') return stop();
  if (command === 'doctor') {
    if (!existsSync(binary)) { process.exitCode = 2; return { ok: false, error: 'Missing debug Companion' }; }
    const temporary = !owned(state());
    try {
      if (temporary) await start(true, await freePort(), join(target, `tdapp-doctor-${randomBytes(6).toString('hex')}`));
      const client = await session(state().data);
      let info; try { info = await client.request('/system'); } finally { await client.close(); }
      const sha = info.body?.build;
      if (!/^[0-9a-f]{40}$/.test(sha)) { process.exitCode = 2; return { ok: false, error: 'Companion has no usable build SHA' }; }
      const expected = head(); process.exitCode = sha === expected ? 0 : 1;
      return { ok: sha === expected, companion_sha: sha, head_sha: expected, source: '/api/v1/system', temporary };
    } finally { if (temporary) await stop(); }
  }
  if (command === 'ui-e2e') return e2e();
  if (command === 'test') { const pnpm = run('pnpm', ['test']), cargo = run('cargo', ['test', '--workspace']); process.exitCode = Number(Boolean(pnpm || cargo)); return { ok: !pnpm && !cargo, pnpm, cargo }; }
  if (command === 'link') {
    if (!args.includes('--mock') && (!existsSync(hardwareLock) || owned(state()))) throw Error(`Real link requires ${hardwareLock} and stop first`);
    const result = spawnSync(join(target, 'debug/td-link.exe'), args, { cwd: app, encoding: 'utf8' });
    if (result.error) throw result.error;
    process.stderr.write(result.stderr); process.exitCode = result.status;
    return { ok: result.status === 0, exit: result.status, results: result.stdout.trim().split(/\r?\n/).filter(Boolean).map(line => JSON.parse(line)) };
  }
  if (!['api', 'events', 'screenshot', 'devcmd'].includes(command)) throw Error('Commands: build doctor start stop api events devcmd ui-e2e screenshot link test');
  if (!owned(state())) throw Error('Start this checkout Companion first');
  const client = await session(state().data);
  try {
    if (command === 'devcmd') {
      const line = args[0];
      if (!line || line.startsWith('--')) throw Error('Pass one console line, quoted');
      const s = state();
      if (!s.mock && !s.deviceConsole) throw Error('Start the real Companion with --device-console first');
      let device = opt('--device');
      if (!device) {
        const list = await client.request('/devices');
        const connected = (list.body ?? []).filter(d => d.link?.state === 'connected').map(d => d.id);
        if (connected.length !== 1) throw Error(`Pass --device; connected Devices: ${connected.join(' ') || 'none'}`);
        device = connected[0];
      }
      const body = { line }, wait = opt('--wait-ms');
      if (wait !== undefined) body.wait_ms = Number(wait);
      const result = await client.request(`/test/devices/${encodeURIComponent(device)}/console`, 'POST', JSON.stringify(body));
      if (!result.ok) { process.exitCode = 2; return { ok: false, device, line, status: result.status, error: result.body?.error }; }
      const reply = result.body.reply;
      process.exitCode = reply?.ok === true ? 0 : 1;
      return { ok: reply?.ok === true, device, line, reply, lines: result.body.lines };
    }
    if (command === 'api') { const result = await client.request(args[1].replace(/^\/api\/v1/, ''), args[0], opt('--body', '')); process.exitCode = result.ok ? 0 : 1; return result; }
    if (command === 'screenshot') {
      const out = opt('--out'); if (!out || !isAbsolute(out)) throw Error('Pass absolute --out PNG path');
      const { chromium } = require('@playwright/test'), browser = await chromium.launch({ channel: 'msedge' });
      try {
        const page = await browser.newPage({ viewport: { width: 1180, height: 760 } });
        await page.addInitScript(token => { window.openTouchDeck = { version: 1, session: async () => token, onSessionChanged() {} }; }, client.token);
        await page.goto(client.origin); await page.getByRole('list', { name: 'Devices' }).waitFor();
        // Each live Device card has drawn its Layout miniature or its "No Layout yet" state (app/ui/src/screens/DevicePreview.tsx).
        await page.waitForFunction(() => [...document.querySelectorAll('ul[aria-label="Devices"] > li.device-figure')]
          .filter((card) => card.querySelector('.status.is-live'))
          .every((card) => card.querySelector('svg.device-scene[data-page], [data-testid="device-empty"]')), null, { timeout: 15000 })
          .catch(() => { throw Error('Home miniatures did not render within 15 s'); });
        mkdirSync(dirname(out), { recursive: true }); await page.screenshot({ path: out, fullPage: true });
        return { ok: true, out, surface: 'Companion embedded React UI' };
      } finally { await browser.close(); }
    }
    const seconds = Number(opt('--seconds', '5'));
    if (!Number.isFinite(seconds) || seconds <= 0 || seconds > 300) throw Error('seconds must be >0 and <=300');
    const frames = await new Promise((resolve, reject) => {
      const frames = [], socket = new WebSocket(`${client.origin.replace('http:', 'ws:')}/api/v1/events`, ['touchdeck.v1', `touchdeck.bearer.${client.token}`]);
      let captureTimer, finished = false;
      const finish = error => {
        if (finished) return;
        finished = true;
        clearTimeout(deadline); clearTimeout(captureTimer);
        socket.close();
        error ? reject(error) : resolve(frames);
      };
      const deadline = setTimeout(() => finish(Error('Events timeout')), (seconds + 10) * 1000);
      socket.onopen = () => {
        if (socket.protocol !== 'touchdeck.v1') return finish(Error('Unexpected events subprotocol'));
        captureTimer = setTimeout(() => finish(), seconds * 1000);
      };
      socket.onmessage = event => {
        try { frames.push(JSON.parse(event.data)); }
        catch { finish(Error('Invalid events JSON')); }
      };
      socket.onclose = event => finish(Error(`Events connection closed early (${event.code})`));
      socket.onerror = () => finish(Error('Events socket failed'));
    });
    return { ok: true, seconds, frames };
  } finally { await client.close(); }
}
try { const result = await main(); if (command === 'doctor' && opt('--out')) save(opt('--out'), result); console.log(JSON.stringify(result)); if (result.ok === false && !process.exitCode) process.exitCode = 1; }
catch (error) { console.log(JSON.stringify({ ok: false, command, error: error.message })); process.exitCode = 2; }
