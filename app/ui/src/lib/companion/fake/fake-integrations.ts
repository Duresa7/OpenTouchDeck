import { fakeMedia, fakeVolume } from './fake-media.ts';
import { fakeThisPc } from './fake-this-pc.ts';
import type { AccountUsage, Delivery, IntegrationSummary, IntegrationView } from '../api.ts';

type Frame = Record<string, unknown> & { type: string };

export interface FakeIntegrationsOptions {
  publish(frame: Frame): void;
  /** CLIProxyAPI starts set up, on and connected to a hub. Otherwise it starts as a fresh install. */
  configured?: boolean;
  /** Delivery per Device at the start. */
  delivery?: Delivery[];
  now?: () => number;
  /** The accounts the hub reports, in place of the two samples. */
  accounts?: (polled_ms: number) => AccountUsage[];
  /** Why the hub is unreachable while it is down. */
  hubMessage?: string;
  /** More on-or-off Integrations, by id, after the built-in ones. */
  extra?: Partial<Record<string, (enabled: boolean) => IntegrationView>>;
}

const json = (status: number, body: unknown) =>
  new Response(status === 204 ? null : JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

const failure = (status: number, code: string, message: string, details: unknown = null) => json(status, { error: { code, message, details } });

const invalid = () => failure(422, 'integration.config_invalid', 'Invalid Integration config.', { issues: [{ message: 'Check the Integration fields.' }] });

/** The Companion reads `integrations.enabled_put` as a typed body, so any other body fails before the Integration sees it. */
const notEnabledBody = () => failure(400, 'request.invalid', 'The request body is invalid.', { part: 'body', reason: 'the body is {"enabled": boolean}' });

const isObject = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);

/** The `enabled` of a body that is exactly `{enabled: boolean}`, or null. */
function enabledBody(body: unknown): boolean | null {
  if (!isObject(body)) return null;
  const keys = Object.keys(body);
  return keys.length === 1 && keys[0] === 'enabled' && typeof body.enabled === 'boolean' ? body.enabled : null;
}

const ID = 'cliproxyapi';
const SECRET = 'management_key';
const DEFAULT_URL = 'http://127.0.0.1:8317';
const REFRESH_FLOOR_MS = 15_000;

const STREAMS: [string, string, string | null][] = [
  ['status', 'text', null],
  ['success', 'numeric', 'requests'],
  ['failed', 'numeric', 'requests'],
  ['recent_10m', 'numeric', 'requests'],
  ['five_hour_pct', 'numeric', '%'],
  ['seven_day_pct', 'numeric', '%'],
  ['five_hour_reset', 'text', null],
  ['seven_day_reset', 'text', null],
  ['plan', 'text', null],
];

function sampleAccounts(polled: number): AccountUsage[] {
  const window = (pct: number, delay: number) => ({ pct, resets_at_ms: polled + delay });
  return [
    { account: 'Work', provider: 'codex', status: 'active', plan: 'pro', success: 128, failed: 2, recent_10m: 6, five_hour: window(42, 7_800_000), seven_day: window(10, 273_600_000), error: null },
    { account: 'Personal', provider: 'claude', status: 'active', plan: 'Claude', success: 64, failed: 0, recent_10m: 5, five_hour: null, seven_day: null, error: "The service refused this account's sign-in (HTTP 403). Sign in to it again in CLIProxyAPI." },
  ];
}

const PRESETS: [string, string][] = [
  ['status', 'Account status'], ['success', 'Requests OK'], ['failed', 'Requests failed'],
  ['recent_10m', 'Requests 10 min'], ['five_hour_pct', '5-hour used'],
  ['seven_day_pct', '7-day used'], ['five_hour_reset', '5-hour reset in'],
  ['seven_day_reset', '7-day reset in'], ['plan', 'Plan'],
];

function originOf(address: string): string | null {
  let url: URL;
  try {
    url = new URL(address.trim());
  } catch {
    return null;
  }
  if ((url.protocol !== 'http:' && url.protocol !== 'https:') || !url.hostname || url.username || url.password || url.search || url.hash) return null;
  return url.origin;
}

interface Settings {
  enabled: boolean;
  url: string;
  interval_s: number;
}

/** A config without `enabled` keeps the stored one, as the Companion does. */
function parseSettings(body: unknown, stored: boolean): Settings | null {
  if (!isObject(body)) return null;
  const fields = new Map<string, unknown>(Object.entries(body));
  const keys = [...fields.keys()].sort().join(',');
  if (keys !== 'enabled,interval_s,url' && keys !== 'interval_s,url') return null;
  const [enabled, url, interval_s] = [fields.has('enabled') ? fields.get('enabled') : stored, fields.get('url'), fields.get('interval_s')];
  if (typeof enabled !== 'boolean' || typeof url !== 'string' || typeof interval_s !== 'number' || !Number.isInteger(interval_s) || interval_s < 0) return null;
  return { enabled, url, interval_s };
}

export function createFakeIntegrations(options: FakeIntegrationsOptions) {
  const { publish, now = Date.now, accounts = sampleAccounts, hubMessage = 'connection refused' } = options;
  const simple = new Map<string, { enabled: boolean; view(enabled: boolean): IntegrationView }>([
    ['this_pc', { enabled: true, view: fakeThisPc }],
    ['volume', { enabled: true, view: fakeVolume }],
    ['media', { enabled: true, view: fakeMedia }],
  ]);
  for (const [id, view] of Object.entries(options.extra ?? {})) if (view) simple.set(id, { enabled: true, view });
  const simpleViews = () => [...simple.values()].map((entry) => entry.view(entry.enabled).summary);
  const configured = options.configured ?? false;
  let settings: Settings = configured ? { enabled: true, url: 'https://hub.example.com', interval_s: 300 } : { enabled: false, url: DEFAULT_URL, interval_s: 300 };
  let bound = originOf(settings.url);
  let keyOrigin: string | null = configured ? bound : null;
  let hubUp = true;
  let revision = 0;
  const initialPoll = now() - 12_000;
  let detail: IntegrationView['detail'] = configured ? { polled_ms: initialPoll, accounts: accounts(initialPoll) } : null;
  let lastRefresh: number | null = null;
  let refreshes = 0;
  let configPuts = 0;
  /** Accepted CLIProxyAPI `integrations.enabled_put` calls, whether or not they changed it. */
  let enabledPuts = 0;
  const delivery = new Map((options.delivery ?? []).map((d) => [d.device, d]));

  const keySet = () => keyOrigin !== null && keyOrigin === bound;

  function summary(): IntegrationSummary {
    const state = !keySet() ? 'not_configured' : hubUp ? 'ok' : 'error';
    return {
      id: ID,
      revision,
      name: 'CLIProxyAPI',
      icon: 'activity',
      enabled: settings.enabled,
      status: { state, message: state === 'error' ? hubMessage : null, delivery: [...delivery.values()] },
      config_schema: [
        { name: 'enabled', kind: 'boolean', secret: false },
        { name: 'url', kind: 'string', secret: false },
        { name: 'interval_s', kind: 'integer', secret: false },
        { name: SECRET, kind: 'string', secret: true },
      ],
      streams: STREAMS.map(([id, kind, unit]) => ({ id, kind, unit, settable: false })),
      actions: ['refresh'],
      presets: PRESETS.map(([id, name]) => ({ id, name, action: { type: 'widget', source: ID, field: id, account: '' } })),
    };
  }

  const view = (): IntegrationView => ({
    summary: summary(), config: { ...settings }, secrets: { [SECRET]: { set: keySet() } },
    detail,
  });

  const changed = () => {
    revision += 1;
    publish({ type: 'integration.status', data: summary() });
  };

  async function body(request: Request): Promise<unknown> {
    try {
      return JSON.parse(await request.text());
    } catch {
      return undefined;
    }
  }

  return {
    list: () => [summary(), ...simpleViews()],

    /** Answers a session-checked request, or returns null when it isn't an Integrations route. */
    async handle(request: Request, path: string): Promise<Response | null> {
      const method = request.method;
      if (path === '/integrations') return method === 'GET' ? json(200, [summary(), ...simpleViews()]) : failure(405, 'route.method_not_allowed', 'Use GET.');
      const match = /^\/integrations\/([^/]+)(?:\/(config|enabled|refresh|secrets\/([^/]+)))?$/.exec(path);
      if (!match) return null;
      const onOff = simple.get(decodeURIComponent(match[1]));
      if (onOff) {
        const part = match[2];
        if (!part) return method === 'GET' ? json(200, onOff.view(onOff.enabled)) : failure(405, 'route.method_not_allowed', 'Use GET.');
        if (part === 'config') {
          if (method !== 'PUT') return failure(405, 'route.method_not_allowed', 'Use PUT.');
          const input = await body(request);
          if (!isObject(input) || Object.keys(input).length > 1) return invalid();
          if ('enabled' in input) {
            if (typeof input.enabled !== 'boolean') return invalid();
            onOff.enabled = input.enabled;
          }
          publish({ type: 'integration.status', data: onOff.view(onOff.enabled).summary });
          return json(200, onOff.view(onOff.enabled));
        }
        if (part === 'enabled') {
          if (method !== 'PUT') return failure(405, 'route.method_not_allowed', 'Use PUT.');
          const enabled = enabledBody(await body(request));
          if (enabled === null) return notEnabledBody();
          if (enabled !== onOff.enabled) {
            onOff.enabled = enabled;
            publish({ type: 'integration.status', data: onOff.view(onOff.enabled).summary });
          }
          return json(200, onOff.view(onOff.enabled));
        }
        if (part === 'refresh') return method === 'POST' ? json(204, null) : failure(405, 'route.method_not_allowed', 'Use POST.');
        return invalid();
      }
      if (decodeURIComponent(match[1]) !== ID) return failure(404, 'integration.not_found', 'The Integration does not exist.');
      const [, , part, field] = match;
      if (!part) return method === 'GET' ? json(200, view()) : failure(405, 'route.method_not_allowed', 'Use GET.');
      if (part === 'config') {
        if (method !== 'PUT') return failure(405, 'route.method_not_allowed', 'Use PUT.');
        const next = parseSettings(await body(request), settings.enabled);
        if (!next) return invalid();
        const url = next.url.trim().replace(/\/+$/, '');
        const origin = originOf(url);
        if (!origin) return invalid();
        const interval_s = Math.max(60, next.interval_s);
        if (next.enabled === settings.enabled && url === settings.url && interval_s === settings.interval_s) return json(200, view());
        if (origin !== bound) keyOrigin = null;
        bound = origin;
        settings = { enabled: next.enabled, url, interval_s };
        configPuts += 1;
        detail = null;
        changed();
        return json(200, view());
      }
      if (part === 'enabled') {
        if (method !== 'PUT') return failure(405, 'route.method_not_allowed', 'Use PUT.');
        const enabled = enabledBody(await body(request));
        if (enabled === null) return notEnabledBody();
        enabledPuts += 1;
        if (enabled !== settings.enabled) {
          settings = { ...settings, enabled };
          detail = null;
          changed();
        }
        return json(200, view());
      }
      if (part === 'refresh') {
        if (method !== 'POST') return failure(405, 'route.method_not_allowed', 'Use POST.');
        if (settings.enabled && (lastRefresh === null || now() - lastRefresh >= REFRESH_FLOOR_MS)) {
          lastRefresh = now();
          refreshes += 1;
          const polled_ms = now();
          detail = { polled_ms, accounts: keySet() && hubUp ? accounts(polled_ms) : detail?.accounts ?? [] };
          changed();
        }
        return json(204, null);
      }
      if (decodeURIComponent(field) !== SECRET) return invalid();
      if (method === 'DELETE') {
        keyOrigin = null;
        detail = null;
        changed();
        return json(204, null);
      }
      if (method !== 'PUT') return failure(405, 'route.method_not_allowed', 'Use PUT or DELETE.');
      const input = await body(request);
      const keys = typeof input === 'object' && input !== null ? Object.keys(input) : [];
      const value = typeof input === 'object' && input !== null && 'value' in input ? input.value : undefined;
      if (keys.length !== 1 || typeof value !== 'string' || !value.trim() || value.length > 4096 || /\p{Cc}/u.test(value)) return invalid();
      keyOrigin = bound;
      detail = null;
      changed();
      return json(200, { hint: 'Set' });
    },

    test(request: Request, path: string): Response {
      if (path === '/test/integrations/record' && request.method === 'GET') return json(200, { settings, keySet: keySet(), refreshes, configPuts, enabledPuts });
      return failure(404, 'route.not_found', 'No test route with that path.');
    },

    setHub(up: boolean) {
      hubUp = up;
      changed();
    },

    deliver(entry: Delivery) {
      delivery.set(entry.device, entry);
      changed();
    },

    get refreshes() {
      return refreshes;
    },
  };
}

export type FakeIntegrations = ReturnType<typeof createFakeIntegrations>;
