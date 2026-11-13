// Drives the real 1.x updater through the bridge to 2.0 against the installed app, for the bridge upgrade check on
// main (.github/workflows/bridge-upgrade.yml there). A loopback server stands in for github.com and its release list,
// the way lifecycle_update.mjs stands in for latest.json.
//
// Usage: node bridge_update.mjs <good|tampered> <current 1.x version> <dir> <result.json>
// <dir>/next.json names the 2.x MSI in <dir>: { "version", "msi", "signature", "updater_public_key" }.
// good: check, download, verify and hand the MSI to the installed bridge.ps1, then exit as the app would.
// tampered: serve the MSI with one byte changed and expect the signature check to refuse it.
import { spawn } from 'node:child_process';
import { readFile, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { release as osRelease } from 'node:os';
import { basename, dirname, join } from 'node:path';

const [scenario, current, dir, resultPath] = process.argv.slice(2);
if (!['good', 'tampered'].includes(scenario) || !current || !dir || !resultPath) {
  throw new Error('Usage: bridge_update.mjs <good|tampered> <current version> <dir> <result.json>');
}
if (process.platform !== 'win32') throw new Error('The bridge runs on Windows only');
const dist = new URL('../electron/dist/', import.meta.url);
const { Updater } = await import(new URL('updates/index.js', dist).href);
const { HELPER, POWERSHELL, windowsBuild } = await import(new URL('updates/bridge.js', dist).href);
const { connect, runtimePath } = await import(new URL('companion.js', dist).href);

const next = JSON.parse(await readFile(join(dir, 'next.json'), 'utf8'));
const msi = await readFile(join(dir, next.msi));
const served = scenario === 'tampered' ? Buffer.from(msi) : msi;
if (scenario === 'tampered') served[served.length >> 1] ^= 0x01;
const tag = `v${next.version}`;

const server = createServer((req, res) => {
  const url = new URL(req.url, 'http://127.0.0.1');
  const base = `http://127.0.0.1:${server.address().port}`;
  const downloads = `${base}/Duresa7/OpenTouchDeck/releases/download/${tag}`;
  if (url.pathname === '/latest.json') {
    // No newer 1.x: the feed a 2.x release carries names the bridge release itself.
    res.setHeader('Content-Type', 'application/json');
    res.end(JSON.stringify({ version: current, platforms: { 'windows-x86_64': { signature: 'unused', url: `${base}/setup.exe` } } }));
  } else if (url.pathname === '/repos/Duresa7/OpenTouchDeck/releases') {
    res.setHeader('Content-Type', 'application/json');
    res.end(JSON.stringify(url.searchParams.get('page') !== '1' ? [] : [{
      tag_name: tag, draft: false, prerelease: false, body: 'bridge upgrade check', published_at: null,
      assets: [
        { name: next.msi, browser_download_url: `${downloads}/${next.msi}`, size: served.length },
        { name: `${next.msi}.sig`, browser_download_url: `${downloads}/${next.msi}.sig`, size: Buffer.byteLength(next.signature) },
      ],
    }]));
  } else if (url.pathname === `/Duresa7/OpenTouchDeck/releases/download/${tag}/${next.msi}`) {
    res.setHeader('Content-Length', served.length);
    res.end(served);
  } else if (url.pathname === `/Duresa7/OpenTouchDeck/releases/download/${tag}/${next.msi}.sig`) {
    res.end(next.signature);
  } else {
    res.statusCode = 404;
    res.end();
  }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;

// The installed app's own paths, as main.ts derives them in the packaged app.
const runtimeDir = dirname(runtimePath());
const installDir = join(process.env.LOCALAPPDATA, 'Programs', 'Open TouchDeck');
const result = { scenario, windows_build: windowsBuild(osRelease(), process.platform) };
const finish = async () => {
  server.close();
  await writeFile(resultPath, JSON.stringify(result, null, 2));
};

const session = await connect(runtimePath(), () => { throw new Error('The installed Companion is not running'); });
const updater = new Updater({
  publicKey: next.updater_public_key, current, runtimeDir,
  feed: `${origin}/latest.json`, releases: `${origin}/repos/Duresa7/OpenTouchDeck/releases`,
  downloads: `${origin}/Duresa7/OpenTouchDeck/releases/download`, allowLoopback: true,
  bridge: {
    windowsBuild: result.windows_build, installDir, dataDir: dirname(runtimeDir),
    helper: join(installDir, 'resources', HELPER),
    powershell: join(process.env.SystemRoot, 'System32', POWERSHELL), pid: process.pid,
  },
  system: () => async (method, path) => {
    const res = await fetch(`${session.origin}/api/v1${path}`, {
      method, redirect: 'error', signal: AbortSignal.timeout(5000),
      headers: { Authorization: `Bearer ${session.token}`, 'Content-Type': 'application/json' },
    });
    return { status: res.status, body: await res.json().catch(() => null) };
  },
  launch: (path, args) => new Promise((resolve, reject) => {
    // Only names go into the result: the paths hold the runner's user name.
    result.launched = { program: basename(path), args: args.map(arg => (arg.includes('\\') ? basename(arg) : arg)) };
    const child = spawn(path, args, { detached: true, stdio: 'ignore', windowsHide: true });
    child.once('error', reject);
    child.once('spawn', () => { result.helper_pid = child.pid; child.unref(); resolve(); });
  }),
  quit: () => { result.quit = true; void finish().then(() => process.exit(0)); },
});
result.check = await updater.check();
result.download = await updater.download(() => {});
if (scenario === 'good' && result.download.state === 'ready') {
  result.install = await updater.install();
}
await finish();
