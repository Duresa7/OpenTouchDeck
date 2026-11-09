import { readFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const AUTH_FILES = '/v0/management/auth-files';
export const API_CALL = '/v0/management/api-call';
export const CODEX_USAGE = 'https://chatgpt.com/backend-api/wham/usage';
export const CLAUDE_USAGE = 'https://api.anthropic.com/api/oauth/usage';
export const DEFAULT_KEY = 'fixture-key-0123';
const DEFAULT_FIXTURE = join(fileURLToPath(new URL('.', import.meta.url)), '..', '..', '..', 'crates', 'td-host', 'tests', 'fixtures', 'auth-files-usage.json');

/** Starts the server on `port`, or on a free one when `port` is 0, and resolves once it listens. */
export async function start({ port = 0, key = DEFAULT_KEY, fixture = DEFAULT_FIXTURE, host = '127.0.0.1' } = {}) {
  const body = readFileSync(fixture, 'utf8');
  const requests = [];
  let fiveHourPercent = 42;
  const server = createServer(async (request, response) => {
    const bearer = request.headers.authorization ?? '';
    const authorized = bearer === `Bearer ${key}`;
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    let json = null;
    try { json = JSON.parse(Buffer.concat(chunks).toString('utf8')); } catch {  }
    requests.push({ at: Date.now(), method: request.method, path: request.url, authorized, headers: { ...request.headers }, body: json });
    const send = (status, payload) => {
      response.writeHead(status, { 'content-type': 'application/json', 'content-length': Buffer.byteLength(payload) });
      response.end(payload);
    };
    if (!((request.method === 'GET' && request.url === AUTH_FILES) || (request.method === 'POST' && request.url === API_CALL))) return send(404, '{"error":"not found"}');
    if (!authorized) return send(401, '{"error":"missing management key"}');
    if (request.url === AUTH_FILES) return send(200, body);
    const now = Math.floor(Date.now() / 1000);
    let usage;
    if (json?.authIndex === 'codex-alice' && json?.url === CODEX_USAGE && json?.method === 'GET') {
      usage = { plan_type: 'pro', rate_limit: {
        primary_window: { used_percent: fiveHourPercent, reset_at: now + 7800, limit_window_seconds: 18000 },
        secondary_window: { used_percent: 10, reset_at: now + 273600, limit_window_seconds: 604800 },
      } };
    } else if (json?.authIndex === 'claude-alice' && json?.url === CLAUDE_USAGE && json?.method === 'GET') {
      usage = { five_hour: { utilization: 24, resets_at: new Date((now + 7800) * 1000).toISOString() },
        seven_day: { utilization: 61, resets_at: new Date((now + 273600) * 1000).toISOString() } };
    } else return send(404, '{"error":"unknown fixture account or URL"}');
    send(200, JSON.stringify({ status_code: 200, header: {}, body: JSON.stringify(usage) }));
  });
  server.keepAliveTimeout = 0;
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(port, host, resolve);
  });
  const address = server.address();
  return {
    url: `http://${host}:${address.port}`,
    port: address.port,
    requests,
    setFiveHourPercent: (percent) => { fiveHourPercent = percent; },
    stop: () =>
      new Promise((resolve) => {
        server.closeAllConnections();
        server.close(() => resolve());
      }),
  };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const option = (name, fallback) => {
    const at = process.argv.indexOf(name);
    return at >= 0 ? process.argv[at + 1] : fallback;
  };
  const server = await start({ port: Number(option('--port', 8317)), key: option('--key', DEFAULT_KEY), fixture: option('--fixture', DEFAULT_FIXTURE) });
  console.log(JSON.stringify({ url: server.url }));
}
