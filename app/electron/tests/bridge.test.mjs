import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash, generateKeyPairSync, randomBytes, sign } from 'node:crypto';
import { createServer } from 'node:http';
import { mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Updater, feedSettings } from '../dist/updates/index.js';
import { bridgeNotice, helperArgs, msiName, selectNextApp, signedFields, windowsBuild } from '../dist/updates/bridge.js';

const DOWNLOADS = 'https://github.com/Duresa7/OpenTouchDeck/releases/download';

function signer() {
  const { publicKey, privateKey } = generateKeyPairSync('ed25519');
  const id = randomBytes(8);
  const key = Buffer.concat([Buffer.from('Ed'), id, publicKey.export({ type: 'spki', format: 'der' }).subarray(-32)]);
  return {
    key: Buffer.from(`untrusted comment: test\n${key.toString('base64')}\n`).toString('base64'),
    sign(data, { version, file }) {
      const sig = sign(null, createHash('blake2b512').update(data).digest(), privateKey);
      const comment = `timestamp:1\tfile:${file}\tversion:${version}`;
      const global = sign(null, Buffer.concat([sig, Buffer.from(comment)]), privateKey);
      const packet = Buffer.concat([Buffer.from('ED'), id, sig]);
      return Buffer.from(`untrusted comment: test\n${packet.toString('base64')}\ntrusted comment: ${comment}\n${global.toString('base64')}\n`).toString('base64');
    },
  };
}

function release(version, { downloads = DOWNLOADS, draft = false, prerelease = false, size = 10, assets } = {}) {
  const tag = `v${version}`;
  const name = msiName(version);
  return { tag_name: tag, draft, prerelease, body: `Notes for ${version}`, published_at: '2026-12-01T00:00:00Z',
    assets: assets ?? [
      { name, browser_download_url: `${downloads}/${tag}/${name}`, size },
      { name: `${name}.sig`, browser_download_url: `${downloads}/${tag}/${name}.sig`, size: 1 },
    ] };
}

test('reads the Windows build from os.release, and nothing elsewhere', () => {
  assert.equal(windowsBuild('10.0.22631', 'win32'), 22631);
  assert.equal(windowsBuild('10.0.19045', 'win32'), 19045);
  assert.equal(windowsBuild('10.0.26100.1742', 'win32'), 26100);
  assert.equal(windowsBuild('6.1.7601', 'win32'), null);
  assert.equal(windowsBuild('10.0.22631', 'linux'), null);
});

test('selects the newest stable 2.x with exactly one MSI and signature at its own release URL', () => {
  const pages = [[release('1.3.0'), release('2.0.0'), release('2.1.0', { prerelease: true }), release('2.0.5'),
    release('2.2.0', { draft: true }), release('3.0.0'), release('2.0.6-rc.1'), { tag_name: 'v2.9.0' }]];
  const next = selectNextApp(pages, 1000);
  assert.deepEqual(next, {
    version: '2.0.5', tag: 'v2.0.5', notes: 'Notes for 2.0.5', date: '2026-12-01T00:00:00Z',
    msi: { name: 'OpenTouchDeck-2.0.5-windows-x64.msi', url: `${DOWNLOADS}/v2.0.5/OpenTouchDeck-2.0.5-windows-x64.msi`, size: 10 },
    signature: { url: `${DOWNLOADS}/v2.0.5/OpenTouchDeck-2.0.5-windows-x64.msi.sig`, size: 1 },
  });
  assert.equal(selectNextApp([[release('1.3.0')], []], 1000), null);
  assert.throws(() => selectNextApp([{}], 1000));
});

test('refuses a chosen release whose assets are missing, doubled, oversized or elsewhere', () => {
  const name = msiName('2.0.0');
  const msi = { name, browser_download_url: `${DOWNLOADS}/v2.0.0/${name}`, size: 10 };
  const sig = { name: `${name}.sig`, browser_download_url: `${DOWNLOADS}/v2.0.0/${name}.sig`, size: 1 };
  for (const assets of [[msi], [sig], [msi, msi, sig], [{ ...msi, size: 0 }, sig], [{ ...msi, size: 1001 }, sig],
    [{ ...msi, browser_download_url: `https://example.invalid/${name}` }, sig], [msi, { ...sig, size: 16 * 1024 + 1 }]]) {
    assert.throws(() => selectNextApp([[release('2.0.0', { assets })]], 1000));
  }
});

test('reads the signed file and version, and builds the helper command line', () => {
  assert.deepEqual(signedFields('timestamp:1\tfile:OpenTouchDeck-2.0.0-windows-x64.msi\tversion:2.0.0'),
    { file: 'OpenTouchDeck-2.0.0-windows-x64.msi', version: '2.0.0' });
  assert.deepEqual(helperArgs({ script: 'C:\\s\\bridge.ps1', msi: 'C:\\s\\a b.msi', sha256: 'ab', installDir: 'C:\\P', dataDir: 'C:\\D', waitPid: 42 }),
    ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', 'C:\\s\\bridge.ps1', '-Msi', 'C:\\s\\a b.msi',
      '-Sha256', 'ab', '-InstallDir', 'C:\\P', '-DataDir', 'C:\\D', '-WaitPid', '42']);
});

test('says why 1.x is still here after a rollback, and nothing otherwise', () => {
  assert.match(bridgeNotice({ status: 'cancelled' }, 'C:\\D\\bridge').message, /cancelled/);
  for (const status of ['failed', 'refused']) {
    const notice = bridgeNotice({ status }, 'C:\\D\\bridge');
    assert.equal(notice.message, "Couldn't update to Open TouchDeck 2.0.");
    assert.match(notice.detail, /C:\\D\\bridge/);
  }
  assert.equal(bridgeNotice({ status: 'moved' }, 'x'), null);
  assert.equal(bridgeNotice(null, 'x'), null);
});

test('an unpackaged build alone points the release list at a loopback stand-in', () => {
  assert.deepEqual(feedSettings(false, { TOUCHDECK_UPDATE_TEST_GITHUB: 'http://127.0.0.1:9/x' }), {
    feed: 'https://github.com/Duresa7/OpenTouchDeck/releases/latest/download/latest.json',
    releases: 'http://127.0.0.1:9/repos/Duresa7/OpenTouchDeck/releases',
    downloads: 'http://127.0.0.1:9/Duresa7/OpenTouchDeck/releases/download', allowLoopback: true,
  });
  const packaged = feedSettings(true, { TOUCHDECK_UPDATE_TEST_GITHUB: 'http://127.0.0.1:9' });
  assert.equal(packaged.releases, 'https://api.github.com/repos/Duresa7/OpenTouchDeck/releases');
  assert.equal(packaged.allowLoopback, false);
  assert.throws(() => feedSettings(false, { TOUCHDECK_UPDATE_TEST_GITHUB: 'http://example.invalid' }));
});

async function fixture(t, { build = 22631, feedVersion = '1.3.0' } = {}) {
  const dir = await mkdtemp(join(tmpdir(), 'touchdeck-bridge-'));
  const signing = signer();
  const msi = randomBytes(32 * 1024);
  const state = { msi, feedVersion, releases: null, signature: null, calls: [], launches: [], quits: 0, requests: [] };
  const server = createServer((req, res) => {
    state.requests.push({ url: req.url, agent: req.headers['user-agent'] });
    const url = new URL(req.url, 'http://x');
    if (url.pathname === '/latest.json') {
      res.end(JSON.stringify({ version: state.feedVersion, platforms: { 'windows-x86_64': { url: `${origin}/setup.exe`, signature: 'x' } } }));
    } else if (url.pathname === '/repos/Duresa7/OpenTouchDeck/releases') {
      if (state.releases === 'error') { res.writeHead(403); res.end('{}'); return; }
      res.end(JSON.stringify(url.searchParams.get('page') === '1' ? state.releases : []));
    } else if (url.pathname.endsWith('.msi.sig')) {
      res.end(state.signature);
    } else if (url.pathname.endsWith('.msi')) {
      res.end(state.msi);
    } else { res.writeHead(404); res.end(); }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const settings = feedSettings(false, { TOUCHDECK_UPDATE_TEST_FEED: `${origin}/latest.json`, TOUCHDECK_UPDATE_TEST_GITHUB: origin });
  state.releases = [release('2.0.0', { downloads: settings.downloads, size: msi.length })];
  state.signature = signing.sign(msi, { version: '2.0.0', file: msiName('2.0.0') });
  const helper = join(dir, 'installed-bridge.ps1');
  await writeFile(helper, '# helper');
  const runtimeDir = join(dir, 'runtime');
  await (await import('node:fs/promises')).mkdir(runtimeDir);
  const options = { publicKey: signing.key, current: '1.3.0', runtimeDir, ...settings,
    bridge: { windowsBuild: build, installDir: 'C:\\Programs\\Open TouchDeck', dataDir: 'C:\\Data', helper, powershell: 'powershell.exe', pid: 4242 },
    system: () => async (method, path) => {
      state.calls.push([method, path]);
      if (method === 'POST' && path === '/system/lock') return { status: 200, body: { lock: 'test-lock' } };
      return { status: 204, body: null };
    },
    launch: async (path, args) => {
      const script = args[args.indexOf('-File') + 1];
      state.launches.push({ path, args, script: await readFile(script, 'utf8'), msi: await readFile(args[args.indexOf('-Msi') + 1]) });
    },
    quit: () => { state.quits++; },
  };
  t.after(async () => { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); await rm(dir, { recursive: true, force: true }); });
  return { dir, runtimeDir, state, options, updater: new Updater(options), signing, origin };
}

test('Windows 11 is offered the newest 2.x when 1.x is up to date', async t => {
  const f = await fixture(t);
  assert.deepEqual(await f.updater.check(), { state: 'available', version: '2.0.0', current: '1.3.0',
    notes: 'Notes for 2.0.0', date: '2026-12-01T00:00:00Z', bridge: true });
  const api = f.state.requests.find(r => r.url.startsWith('/repos/'));
  assert.equal(api.url, '/repos/Duresa7/OpenTouchDeck/releases?per_page=100&page=1');
  assert.equal(api.agent, 'OpenTouchDeck-updater');
});

test('Windows 10 is never offered 2.x and learns that it needs Windows 11', async t => {
  const f = await fixture(t, { build: 19045 });
  assert.deepEqual(await f.updater.check(), { state: 'none', needsWindows11: '2.0.0' });
  assert.deepEqual(await f.updater.download(() => {}), { state: 'failed', reason: 'download' });
  const unknown = await fixture(t, { build: null });
  assert.deepEqual(await unknown.updater.check(), { state: 'none', needsWindows11: '2.0.0' });
});

test('a newer 1.x comes before 2.x, so a fix to the bridge arrives first', async t => {
  const f = await fixture(t, { feedVersion: '1.3.1' });
  assert.equal((await f.updater.check()).version, '1.3.1');
  assert.equal(f.state.requests.some(r => r.url.startsWith('/repos/')), false);
});

test('a failed 2.x lookup leaves the 1.x result, and a build without the bridge never looks', async t => {
  const f = await fixture(t);
  f.state.releases = 'error';
  assert.deepEqual(await f.updater.check(), { state: 'none' });
  f.state.releases = [release('1.3.0')];
  assert.deepEqual(await f.updater.check(), { state: 'none' });
  const plain = await fixture(t);
  assert.deepEqual(await new Updater({ ...plain.options, bridge: undefined }).check(), { state: 'none' });
  assert.equal(plain.state.requests.some(r => r.url.startsWith('/repos/')), false);
});

for (const failure of ['flipped', 'file', 'version']) test(`refuses a 2.x MSI with a ${failure} mismatch and deletes it`, async t => {
  const f = await fixture(t);
  if (failure === 'flipped') f.state.msi[100] ^= 1;
  if (failure === 'file') f.state.signature = f.signing.sign(f.state.msi, { version: '2.0.0', file: 'setup.exe' });
  if (failure === 'version') f.state.signature = f.signing.sign(f.state.msi, { version: '2.0.1', file: msiName('2.0.0') });
  await f.updater.check();
  assert.deepEqual(await f.updater.download(() => {}), { state: 'failed', reason: 'signature' });
  assert.deepEqual(await readdir(f.runtimeDir), []);
  assert.deepEqual(await f.updater.install(), { state: 'failed', reason: 'installer' });
  assert.equal(f.state.launches.length, 0);
});

test('hands a verified MSI to bridge.ps1 with its SHA-256 after stopping the Companion, then quits', async t => {
  const f = await fixture(t);
  await f.updater.check();
  assert.deepEqual(await f.updater.download(() => {}), { state: 'ready' });
  let resolved = false; void f.updater.install().then(() => { resolved = true; });
  for (let n = 0; n < 200 && !f.state.quits; n++) await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(f.state.quits, 1); assert.equal(resolved, false);
  assert.deepEqual(f.state.calls, [['POST', '/system/lock'], ['PUT', '/system/lock/test-lock'], ['POST', '/system/shutdown']]);
  const [launch] = f.state.launches;
  assert.equal(launch.path, 'powershell.exe');
  assert.equal(launch.script, '# helper');
  assert.deepEqual(launch.msi, f.state.msi);
  const arg = name => launch.args[launch.args.indexOf(name) + 1];
  assert.equal(arg('-Sha256'), createHash('sha256').update(f.state.msi).digest('hex'));
  assert.match(arg('-Msi'), /OpenTouchDeck-2\.0\.0-windows-x64\.msi$/);
  assert.equal(arg('-InstallDir'), 'C:\\Programs\\Open TouchDeck');
  assert.equal(arg('-DataDir'), 'C:\\Data');
  assert.equal(arg('-WaitPid'), '4242');
});

test('an MSI changed after the download is refused before the Companion stops', async t => {
  const f = await fixture(t);
  await f.updater.check(); await f.updater.download(() => {});
  const staging = join(f.runtimeDir, (await readdir(f.runtimeDir))[0]);
  await writeFile(join(staging, msiName('2.0.0')), 'tampered');
  assert.deepEqual(await f.updater.install(), { state: 'failed', reason: 'installer' });
  assert.deepEqual(f.state.calls, []);
  assert.deepEqual(await readdir(f.runtimeDir), []);
});
