// Talks to the installed 1.x Companion the way the 1.x window does, with the session code from a 1.x checkout.
//
// Usage: node onex_api.mjs <1.x checkout> <seed|read> <out.json>
//   seed  stores the CLIProxyAPI Integration's settings and its management key through 1.x's own API, so 1.x writes
//         them to sources.json and Windows Credential Manager itself.
//   read  writes what 1.x reports: its Devices, the Layout it last saw on each, its Pairings, the Integration and
//         start at sign-in.
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

export const INTEGRATION = { enabled: false, url: 'http://127.0.0.1:8318', interval_s: 300 };
export const MANAGEMENT_KEY = 'bridge-test-management-key';

const [checkout, command, out] = process.argv.slice(2);
if (!checkout || !['seed', 'read'].includes(command) || !out) throw new Error('Usage: onex_api.mjs <1.x checkout> <seed|read> <out.json>');
const { connect, runtimePath } = await import(pathToFileURL(join(checkout, 'app/electron/dist/companion.js')).href);
const session = await connect(runtimePath(), () => { throw new Error('The 1.x Companion is not running'); });

async function call(method, path, body) {
  const res = await fetch(`${session.origin}/api/v1${path}`, {
    method, redirect: 'error', signal: AbortSignal.timeout(10_000),
    headers: { Authorization: `Bearer ${session.token}`, 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${method} ${path}: ${res.status} ${text}`);
  return text ? JSON.parse(text) : null;
}

const result = {};
if (command === 'seed') {
  result.config = (await call('PUT', '/integrations/cliproxyapi/config', INTEGRATION)).config;
  await call('PUT', '/integrations/cliproxyapi/secrets/management_key', { value: MANAGEMENT_KEY });
} else {
  result.devices = [];
  for (const device of await call('GET', '/devices')) {
    let layout = null;
    try { layout = (await call('GET', `/devices/${device.id}/layouts/landscape/last-seen`)).layout; } catch { }
    result.devices.push({ id: device.id, name: device.name, setup: device.setup, layout_name: layout?.name ?? null, layout_pages: layout?.pages.length ?? null });
  }
  result.pairings = (await call('GET', '/wifi/pairings')).map(pairing => ({ device: pairing.device, name: pairing.name }));
  const integration = await call('GET', '/integrations/cliproxyapi');
  result.integration = { config: integration.config, secrets: integration.secrets };
  result.startup = await call('GET', '/system/startup');
}
await writeFile(out, JSON.stringify(result, null, 2));
