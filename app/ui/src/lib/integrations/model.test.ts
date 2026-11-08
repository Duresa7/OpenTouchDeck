import { createFakeIntegrations } from '../companion/fake.ts';
import type { IntegrationView } from '../companion/api.ts';
import { describe, expect, it } from 'vitest';
import type { ConfigField, DataStream, IntegrationSummary } from '../companion/api.ts';
import {
  ago,
  badgeOf,
  countsLine,
  formatCount,
  formOf,
  headerLine,
  outcomeAt,
  settingsTitle,
  offersPresets,
  parseAction,
  lineOf,
  parseUsage,
  presetLabel,
  presetLook,
  resetLine,
  serialize,
  statusOf,
  transportOf,
  transportWarning,
  usageLine,
  validate,
  valuesOf,
} from './model.ts';

const SCHEMA: ConfigField[] = [
  { name: 'enabled', kind: 'boolean', secret: false },
  { name: 'url', kind: 'string', secret: false },
  { name: 'interval_s', kind: 'integer', secret: false },
  { name: 'management_key', kind: 'string', secret: true },
];

const summary = (state: string, enabled = true, message: string | null = null): Pick<IntegrationSummary, 'enabled' | 'status'> => ({ enabled, status: { state, message, delivery: [] } });

describe('the form a schema builds', () => {
  it('gives each field its control kind and the copy guide label, and keeps secret fields apart', () => {
    const { fields, secrets } = formOf('cliproxyapi', SCHEMA);
    expect(fields.map((f) => [f.name, f.kind, f.label])).toEqual([
      ['url', 'string', 'Hub address'],
      ['interval_s', 'integer', 'Check every'],
    ]);
    expect(fields[0].placeholder).toBe('https://hub.example.com');
    expect(fields[1].unit).toBe('s');
    expect(secrets.map((f) => [f.name, f.label])).toEqual([['management_key', 'Management key']]);
  });

  it('names the fields of an Integration the copy guide lacks from the schema, and reads an unknown kind as text', () => {
    const { fields, secrets } = formOf('this_pc', [
      { name: 'node_name', kind: 'string', secret: false },
      { name: 'poll_every', kind: 'duration', secret: false },
      { name: 'api_token', kind: 'string', secret: true },
    ]);
    expect(fields.map((f) => [f.label, f.kind])).toEqual([
      ['Node name', 'string'],
      ['Poll every', 'string'],
    ]);
    expect(secrets[0].label).toBe('Api token');
  });

  it('starts from the config the Companion holds', () => {
    const { fields } = formOf('cliproxyapi', SCHEMA);
    expect(valuesOf(fields, { enabled: true, url: 'https://hub.example.com', interval_s: 300 })).toEqual({ url: 'https://hub.example.com', interval_s: '300' });
    expect(valuesOf(fields, null)).toEqual({ url: '', interval_s: '' });
  });

  it.each([
    ['300', undefined],
    [' 60 ', undefined],
    ['', 'Enter a whole number.'],
    ['1.5', 'Enter a whole number.'],
    ['-5', 'Enter a whole number.'],
    ['ten', 'Enter a whole number.'],
  ])('checks an integer field holding %j', (interval, message) => {
    const { fields } = formOf('cliproxyapi', SCHEMA);
    expect(validate(fields, { enabled: true, url: '', interval_s: interval }).interval_s).toBe(message);
  });

  it('sends only the schema settings, typed, and never a secret field even when one is in the values', () => {
    const { fields } = formOf('cliproxyapi', SCHEMA);
    const config = serialize(fields, { enabled: true, url: 'https://hub.example.com', interval_s: '120', management_key: 'sk-never-sent' });
    expect(config).toEqual({ url: 'https://hub.example.com', interval_s: 120 });
    expect(JSON.stringify(config)).not.toContain('sk-never-sent');
  });

  it('leaves the switch out of the form, so Save never carries enabled', () => {
    const { fields, secrets } = formOf('volume', [{ name: 'enabled', kind: 'boolean', secret: false }]);
    expect(fields).toEqual([]);
    expect(secrets).toEqual([]);
    expect(serialize(formOf('cliproxyapi', SCHEMA).fields, { enabled: false, url: 'https://hub.example.com', interval_s: '300' })).not.toHaveProperty('enabled');
  });

  it('heads the form for what it holds and puts each outcome under the part it came from', () => {
    expect(settingsTitle('cliproxyapi')).toBe('Connection');
    expect(settingsTitle('custom')).toBe('Sources');
    expect(settingsTitle('elsewhere')).toBe('Settings');
    expect(['saved', 'save-failed', 'secret-forgotten', 'saving'].map(outcomeAt)).toEqual(['settings', 'settings', 'settings', 'settings']);
    expect(['secret-saved', 'secret-failed', 'forget-failed'].map(outcomeAt)).toEqual(['secret', 'secret', 'secret']);
    expect(['refreshed', 'refresh-floor', 'refresh-failed'].map(outcomeAt)).toEqual(['status', 'status', 'status']);
    expect(outcomeAt('load-failed')).toBe('page');
  });
});

describe('how a key travels to an address', () => {
  it.each([
    ['https://hub.example.com', 'secure'],
    ['http://127.0.0.1:8317', 'local'],
    ['http://localhost:8317', 'local'],
    ['http://[::1]:8317', 'local'],
    ['http://192.168.1.20:8317', 'unencrypted'],
    ['http://10.0.0.5', 'unencrypted'],
    ['http://172.20.1.1', 'unencrypted'],
    ['http://[fd00::1]', 'unencrypted'],
    ['http://172.40.1.1', 'refused'],
    ['http://hub.example.com', 'refused'],
    ['ftp://hub.example.com', 'invalid'],
    ['not an address', 'invalid'],
  ])('%s is %s', (address, transport) => {
    expect(transportOf(address)).toBe(transport);
  });

  it('warns only when the key would go unencrypted or not at all', () => {
    expect(transportWarning('https://hub.example.com')).toBeNull();
    expect(transportWarning('http://127.0.0.1:8317')).toBeNull();
    expect(transportWarning('http://192.168.1.20')).toBe("This address isn't on this PC and uses http://, so the key is sent unencrypted. Use https://.");
    expect(transportWarning('http://hub.example.com')).toBe("This address uses http:// and isn't on your network, so the key isn't sent. Use https://.");
  });
});

describe('status', () => {
  it.each([
    ['not_configured', true, 'Finish setup', 'wait', false, true],
    ['not_configured', false, 'Not set up', null, false, true],
    ['ok', false, 'Off', null, false, false],
    ['error', false, 'Off', null, false, false],
    ['ok', true, 'Connected', 'ok', true, false],
    ['error', true, "Can't connect", 'bad', false, false],
    ['warming', true, 'Checking…', 'wait', false, false],
  ] as const)('%s while %s reads %s with a %s box', (state, enabled, words, tone, live, needsSetup) => {
    expect(statusOf(summary(state, enabled))).toEqual(expect.objectContaining({ words, tone, live, needsSetup }));
  });

  it('keeps the raw reason for Details', () => {
    expect(statusOf(summary('error', true, 'connection refused')).detail).toBe('connection refused');
  });

  it('offers Preset tiles only from an Integration that is set up and on', () => {
    const presets = [{ id: 'plan', name: 'plan', action: { type: 'widget', source: 'cliproxyapi', field: 'plan', account: '' } }];
    const base = { revision: 0, id: 'x', name: 'X', icon: '', config_schema: [], streams: [], actions: [], presets };
    expect(offersPresets({ ...base, ...summary('ok') })).toBe(true);
    expect(offersPresets({ ...base, ...summary('error') })).toBe(true);
    expect(offersPresets({ ...base, ...summary('ok', false) })).toBe(false);
    expect(offersPresets({ ...base, ...summary('not_configured') })).toBe(false);
    expect(offersPresets({ ...base, ...summary('ok'), presets: [] })).toBe(false);
  });
});

describe('usage', () => {
  const NOW = Date.UTC(2026, 9, 29, 12);
  const account = { key: 'codex:alice@example.com', provider: 'codex', status: 'active', plan: 'pro', success: 1, failed: 2, recent_10m: 3, five_hour: { pct: 42, reset_in: '2h' }, seven_day: null, usage_error: null };

  it('maps typed account detail to the Accounts table', async () => {
    const fake = createFakeIntegrations({ configured: true, publish: () => {} });
    const response = await fake.handle(new Request('http://localhost/integrations/cliproxyapi'), '/integrations/cliproxyapi');
    if (!response) throw new Error('Missing view');
    const view: IntegrationView = await response.json();
    expect(parseUsage(view)?.accounts[0]).toMatchObject({ key: 'codex:Work', five_hour: { pct: 42, reset_in: '2h 10m' } });
  });

  it('describes the last checks', () => {
    const usage = { polled_ms: NOW - 12_000, fails: 0, stale: false, accounts: [account] };
    expect(usageLine(usage, NOW)).toBe('1 account · checked 12 s ago');
    expect(usageLine({ ...usage, fails: 2 }, NOW)).toBe('The hub missed the last 2 checks. Retrying.');
    expect(usageLine({ ...usage, fails: 3, stale: true }, NOW)).toBe("Can't connect. The hub missed 3 checks, so its Tiles are greyed out.");
    expect(usageLine({ ...usage, polled_ms: null }, NOW)).toBe('Checking…');
  });

  it('says beside the status box what the box does not', () => {
    const usage = { polled_ms: NOW - 12_000, fails: 0, stale: false, accounts: [account, account] };
    expect(headerLine(statusOf(summary('ok')), usage, NOW)).toBe('2 accounts · checked 12 s ago');
    expect(headerLine(statusOf(summary('ok')), null, NOW)).toBe('Running');
    expect(headerLine(statusOf(summary('error')), usage, NOW)).toBe("Can't connect");
    expect(headerLine(statusOf(summary('not_configured')), null, NOW)).toBe('Finish setup');
    expect(headerLine(statusOf(summary('warming')), null, NOW)).toBe('Checking…');
  });

  it('words times, resets and badges as the copy guide does', () => {
    expect(ago(null, NOW)).toBe('Never');
    expect(ago(NOW - 5_000, NOW)).toBe('5 s ago');
    expect(ago(NOW - 10 * 60_000, NOW)).toBe('10 min ago');
    expect(ago(NOW - 3 * 3_600_000, NOW)).toBe('3 hr ago');
    expect(resetLine({ pct: 1, reset_in: 'now' })).toBe('Resets now');
    expect(resetLine({ pct: 1, reset_in: '2h 10m' })).toBe('Resets in 2h 10m');
    expect(resetLine({ pct: 1, reset_in: null })).toBe('Reset time unknown');
    expect(['active', 'disabled', 'unavailable', 'error', 'paused', 'quota_exhausted'].map(badgeOf)).toEqual(['Active', 'Disabled', 'Unavailable', 'Error', 'Paused', 'Quota exhausted']);
  });

  it('groups the digits of huge counts and rounds a share', () => {
    const grouped = new Intl.NumberFormat();
    const account = { key: 'k', provider: 'p', status: 'active', plan: null, success: 1_284_567_890, failed: -1, recent_10m: 9_999_999, five_hour: null, seven_day: null, usage_error: null };
    expect(countsLine(account)).toBe(`${grouped.format(1_284_567_890)} OK · ${grouped.format(-1)} failed · ${grouped.format(9_999_999)} in 10 min`);
    expect(formatCount(999_999_999.4)).toBe(grouped.format(999_999_999));
    expect(formatCount(-12.5)).toBe(grouped.format(-12));
  });
});

describe('Preset tiles', () => {
  const streams: DataStream[] = [
    { id: 'five_hour_pct', kind: 'numeric', unit: '%', settable: false },
    { id: 'success', kind: 'numeric', unit: 'requests', settable: false },
    { id: 'plan', kind: 'text', unit: null, settable: false },
  ];
  const preset = (field: string) => ({ id: field, name: field.replaceAll('_', ' '), action: { type: 'widget', source: 'cliproxyapi', field, account: '' } });

  it('draws a share as a bar, a count as a graph and anything else as text', () => {
    expect(presetLook(preset('five_hour_pct'), streams)).toBe('bar');
    expect(presetLook(preset('success'), streams)).toBe('graph');
    expect(presetLook(preset('plan'), streams)).toBe('text');
    expect(presetLook(preset('unknown'), streams)).toBe('text');
  });

  it('draws a Preset by its renderer: a level for a bar, and the cover and controls for now playing', () => {
    expect(presetLook({ ...preset('five_hour_pct'), render: 'bar' }, streams)).toBe('level');
    expect(presetLook({ ...preset('plan'), render: 'now-playing' }, streams)).toBe('now-playing');
    expect(presetLook({ ...preset('success'), render: 'graph' }, streams)).toBe('graph');
  });

  it('gives Volume and Media one line each', () => {
    expect(lineOf('volume')).toBe("Shows and sets this PC's volume.");
    expect(lineOf('media')).toBe("Shows what's playing on this PC, with play, pause and skip.");
  });

  it('labels a Tile in sentence case within 16 characters', () => {
    expect(presetLabel('five hour pct')).toBe('Five hour pct');
    expect(presetLabel('a very long preset tile name')).toBe('A very long pres');
    expect(presetLabel('   ')).toBe('Tile');
    expect(presetLabel('Kitchen tv 👩‍👩‍👧‍👦')).toBe('Kitchen tv');
  });

  it('takes an action the Layout rules accept and refuses one they do not', () => {
    expect(parseAction({ type: 'widget', source: 'cliproxyapi', field: 'plan', account: '' })).toEqual({ type: 'widget', source: 'cliproxyapi', field: 'plan', account: '' });
    expect(parseAction({ type: 'widget', source: '', field: 'plan', account: '' })).toBeNull();
    expect(parseAction({ type: 'teleport' })).toBeNull();
    expect(parseAction('widget')).toBeNull();
  });
});

it('round-trips a JSON config field and rejects malformed JSON', () => {
  const { fields } = formOf('custom', [{ name: 'sources', kind: 'json', secret: false }]);
  const sources = [{ id: 'temperature', interval_s: 1 }];
  const values = valuesOf(fields, { sources });
  expect(serialize(fields, values)).toEqual({ sources });
  expect(validate(fields, { sources: '[broken' })).toEqual({ sources: 'Enter valid JSON.' });
});
