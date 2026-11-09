import type { IntegrationView, UsageWindow as ApiUsageWindow } from '../companion/api.ts';
import type { ConfigField, DataStream, IntegrationSummary, JsonValue, PresetTile } from '../companion/api.ts';
import { plural } from '../layout/messages.ts';
import { parseLayout, type Action } from '../layout/model.ts';

interface FieldCopy {
  label: string;
  placeholder?: string;
  unit?: string;
}

interface IntegrationCopy {
  line: string;
  fields: Partial<Record<string, FieldCopy>>;
}

const COPY: Partial<Record<string, IntegrationCopy>> = {
  volume: { line: "Shows and sets this PC's volume.", fields: {} },
  media: { line: "Shows what's playing on this PC, with play, pause and skip.", fields: {} },
  this_pc: { line: "Shows this PC's CPU, memory, GPU, disk and network use.", fields: {} },
  custom: { line: 'Read values from HTTP or a command approved on this PC.', fields: { sources: { label: 'Custom sources (JSON)', placeholder: '[]' } } },
  cliproxyapi: {
    line: "Shows how much of each AI account's limit is used, read from your CLIProxyAPI hub.",
    fields: {
      url: { label: 'Hub address', placeholder: 'https://hub.example.com' },
      interval_s: { label: 'Check every', unit: 's' },
      management_key: { label: 'Management key' },
    },
  },
};

const sentence = (text: string) => text.charAt(0).toUpperCase() + text.slice(1);

const humanize = (name: string) => sentence(name.replaceAll('_', ' ').trim());

/** The one line under an Integration's name. */
export const lineOf = (integration: string) => COPY[integration]?.line ?? '';

export type FieldKind = 'boolean' | 'integer' | 'number' | 'string' | 'json';

export interface FormField {
  name: string;
  kind: FieldKind;
  label: string;
  placeholder?: string;
  unit?: string;
}

const KINDS: readonly string[] = ['boolean', 'integer', 'number', 'string', 'json'];
const isFieldKind = (kind: string): kind is FieldKind => KINDS.includes(kind);

/** The switch's field. It never joins the form: the switch writes it, and Save leaves it as the Companion has it. */
export const ENABLED = 'enabled';

/** The form an Integration's schema builds: its settings, then its write-only secret fields. Never `enabled`. */
export function formOf(integration: string, schema: readonly ConfigField[]): { fields: FormField[]; secrets: FormField[] } {
  const field = (f: ConfigField): FormField => {
    const copy = COPY[integration]?.fields[f.name];
    return { name: f.name, kind: isFieldKind(f.kind) ? f.kind : 'string', label: copy?.label ?? humanize(f.name), placeholder: copy?.placeholder, unit: copy?.unit };
  };
  const settings = schema.filter((f) => f.name !== ENABLED);
  return { fields: settings.filter((f) => !f.secret).map(field), secrets: settings.filter((f) => f.secret).map(field) };
}

const SECTIONS: Partial<Record<string, string>> = { cliproxyapi: 'Connection', custom: 'Sources' };

/** The heading over an Integration's settings form. */
export const settingsTitle = (integration: string) => SECTIONS[integration] ?? 'Settings';

/** Which part of the settings page an outcome belongs under: the settings form, a secret, the checks or the page. */
export function outcomeAt(event: string): 'settings' | 'secret' | 'status' | 'page' {
  if (event === 'saving' || event === 'saved' || event === 'save-failed' || event === 'secret-forgotten') return 'settings';
  if (event.startsWith('secret') || event === 'forget-failed') return 'secret';
  if (event.startsWith('refresh')) return 'status';
  return 'page';
}

/** What the form's controls hold: a switch holds a boolean, every other field the text typed in it. */
export type FormValues = Partial<Record<string, string | boolean>>;

const isRecord = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);

/** The form's starting values, from the config the Companion holds. */
export function valuesOf(fields: readonly FormField[], config: JsonValue): FormValues {
  const source = isRecord(config) ? config : {};
  const values: FormValues = {};
  for (const field of fields) {
    const value = source[field.name];
    if (field.kind === 'boolean') values[field.name] = value === true;
    else if (field.kind === 'json') values[field.name] = JSON.stringify(value ?? [], null, 2);
    else if (typeof value === 'number' || typeof value === 'string') values[field.name] = String(value);
    else values[field.name] = '';
  }
  return values;
}

const text = (values: FormValues, name: string) => {
  const value = values[name];
  return typeof value === 'string' ? value.trim() : '';
};

/** Each field the schema's kinds refuse, with the sentence to show under it. */
export function validate(fields: readonly FormField[], values: FormValues): Partial<Record<string, string>> {
  const errors: Partial<Record<string, string>> = {};
  for (const field of fields) {
    const value = text(values, field.name);
    if (field.kind === 'json') {
      try { JSON.parse(value); } catch { errors[field.name] = 'Enter valid JSON.'; }
    }
    if (field.kind === 'integer' && !/^\d+$/.test(value)) errors[field.name] = 'Enter a whole number.';
    if (field.kind === 'number' && (value === '' || !Number.isFinite(Number(value)))) errors[field.name] = 'Enter a number.';
  }
  return errors;
}

/** The config to send. Only the schema's settings go in it, never a secret field. */
export function serialize(fields: readonly FormField[], values: FormValues): Record<string, JsonValue> {
  const config: Record<string, JsonValue> = {};
  for (const field of fields) {
    if (field.kind === 'boolean') config[field.name] = values[field.name] === true;
    else if (field.kind === 'json') {
      const parsed: JsonValue = JSON.parse(text(values, field.name));
      config[field.name] = parsed;
    }
    else if (field.kind === 'integer' || field.kind === 'number') config[field.name] = Number(text(values, field.name));
    else {
      const value = values[field.name];
      config[field.name] = typeof value === 'string' ? value : '';
    }
  }
  return config;
}

export const sameValues = (fields: readonly FormField[], a: FormValues, b: FormValues) => fields.every((f) => a[f.name] === b[f.name]);

/**
 * How a key would travel to an address, as the Companion decides it: https always, plain http only to this PC or a
 * private network address typed as an IP, and never to anything else.
 */
export type Transport = 'secure' | 'local' | 'unencrypted' | 'refused' | 'invalid';

function privateHost(host: string): boolean {
  const v4 = /^(\d+)\.(\d+)\.(\d+)\.(\d+)$/.exec(host);
  if (v4) {
    const [a, b] = [Number(v4[1]), Number(v4[2])];
    return a === 10 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168);
  }
  return /^\[f[cd][0-9a-f]{0,2}:/i.test(host);
}

const loopbackHost = (host: string) => host === 'localhost' || host === '[::1]' || /^127\.\d+\.\d+\.\d+$/.test(host);

export function transportOf(address: string): Transport {
  let url: URL;
  try {
    url = new URL(address.trim());
  } catch {
    return 'invalid';
  }
  if (url.protocol === 'https:') return 'secure';
  if (url.protocol !== 'http:') return 'invalid';
  if (loopbackHost(url.hostname)) return 'local';
  return privateHost(url.hostname) ? 'unencrypted' : 'refused';
}

/** The warning for an address a key travels to, or null when it travels safely. */
export function transportWarning(address: string): string | null {
  const transport = transportOf(address);
  switch (transport) {
    case 'unencrypted':
      return "This address isn't on this PC and uses http://, so the key is sent unencrypted. Use https://.";
    case 'refused':
      return "This address uses http:// and isn't on your network, so the key isn't sent. Use https://.";
    case 'secure':
    case 'local':
    case 'invalid':
      return null;
    default: {
      const exhaustive: never = transport;
      return exhaustive;
    }
  }
}

/** A status box's colour: green works, red doesn't, amber is on its way or waits for you. */
export type StatusTone = 'ok' | 'bad' | 'wait';

export interface Status {
  words: string;
  /** The status box to show, or null when there's nothing to say beyond the switch: the Integration is off. */
  tone: StatusTone | null;
  /** Connected, so its values are live. */
  live: boolean;
  /** No credentials yet. */
  needsSetup: boolean;
  /** The Companion's raw reason, for Details. */
  detail: string | null;
}

/** The status to show. `pending` is a switch turned on before setup: it reads as on and waiting for setup. */
export function statusOf(summary: Pick<IntegrationSummary, 'enabled' | 'status'>, pending = false): Status {
  const { state, message } = summary.status;
  if (pending && !summary.enabled) return { words: 'Finish setup', tone: 'wait', live: false, needsSetup: state === 'not_configured', detail: null };
  if (state === 'not_configured') return summary.enabled ? { words: 'Finish setup', tone: 'wait', live: false, needsSetup: true, detail: null } : { words: 'Not set up', tone: null, live: false, needsSetup: true, detail: null };
  if (!summary.enabled) return { words: 'Off', tone: null, live: false, needsSetup: false, detail: null };
  if (state === 'ok') return { words: 'Connected', tone: 'ok', live: true, needsSetup: false, detail: null };
  if (state === 'error') return { words: "Can't connect", tone: 'bad', live: false, needsSetup: false, detail: message };
  return { words: 'Checking…', tone: 'wait', live: false, needsSetup: false, detail: message };
}

/** Whether an Integration is set up and on, so the Add Tile panel offers its Preset tiles. */
export const offersPresets = (summary: IntegrationSummary) => summary.enabled && summary.status.state !== 'not_configured' && summary.presets.length > 0;

export function ago(unixMs: number | null, now: number): string {
  if (unixMs === null) return 'Never';
  const seconds = Math.max(0, Math.round((now - unixMs) / 1000));
  if (seconds < 90) return `${seconds} s ago`;
  const minutes = Math.round(seconds / 60);
  return minutes < 90 ? `${minutes} min ago` : `${Math.round(minutes / 60)} hr ago`;
}

export interface UsageWindow {
  pct: number;
  /** `now`, a time such as `2h 10m`, or null when the hub doesn't say. */
  reset_in: string | null;
}

export interface UsageAccount {
  key: string;
  provider: string;
  status: string;
  plan: string | null;
  success: number;
  failed: number;
  recent_10m: number;
  five_hour: UsageWindow | null;
  seven_day: UsageWindow | null;
  usage_error: string | null;
}

export interface Usage {
  polled_ms: number | null;
  fails: number;
  stale: boolean;
  accounts: UsageAccount[];
}

export function parseUsage(view: IntegrationView): Usage | null {
  const { detail } = view;
  if (!detail) return null;
  const window = (value: ApiUsageWindow | null): UsageWindow | null => {
    if (!value) return null;
    const minutes = value.resets_at_ms === null ? null : Math.max(0, Math.ceil((value.resets_at_ms - detail.polled_ms) / 60_000));
    const reset_in = minutes === null ? null : minutes === 0 ? 'now' : minutes >= 1440 ? `${Math.floor(minutes / 1440)}d ${Math.floor(minutes % 1440 / 60)}h` : minutes >= 60 ? `${Math.floor(minutes / 60)}h ${minutes % 60}m` : `${minutes}m`;
    return { pct: value.pct, reset_in };
  };
  return {
    polled_ms: detail.polled_ms,
    fails: 0,
    stale: view.summary.status.state === 'error',
    accounts: detail.accounts.map(account => ({
      ...account,
      key: `${account.provider}:${account.account}`,
      five_hour: window(account.five_hour),
      seven_day: window(account.seven_day),
      usage_error: account.error,
    })),
  };
}

/** The status line above the accounts, from the last checks. */
export function usageLine(usage: Usage, now: number): string {
  if (usage.polled_ms === null && usage.fails === 0) return 'Checking…';
  if (usage.stale) return `Can't connect. The hub missed ${plural(usage.fails, 'check')}, so its Tiles are greyed out.`;
  if (usage.fails > 0) return `The hub missed the last ${plural(usage.fails, 'check')}. Retrying.`;
  return `${plural(usage.accounts.length, 'account')} · checked ${ago(usage.polled_ms, now).toLowerCase()}`;
}

/** The words beside the status box at the top of a settings page. The box already says connected. */
export function headerLine(status: Status, usage: Usage | null, now: number): string {
  if (status.tone === 'ok') return usage ? usageLine(usage, now) : 'Running';
  return status.words;
}

const BADGES: Partial<Record<string, string>> = { active: 'Active', disabled: 'Disabled', unavailable: 'Unavailable', error: 'Error' };

export const badgeOf = (status: string) => BADGES[status.toLowerCase()] ?? humanize(status);

export function resetLine(window: UsageWindow): string {
  if (window.reset_in === 'now') return 'Resets now';
  return window.reset_in ? `Resets in ${window.reset_in}` : 'Reset time unknown';
}

const COUNT = new Intl.NumberFormat();

/** A count or a rounded share with the reader's digit grouping: 1,284,567,890. */
export const formatCount = (value: number) => COUNT.format(Math.round(value));

export const countsLine = (account: UsageAccount) =>
  `${formatCount(account.success)} OK · ${formatCount(account.failed)} failed · ${formatCount(account.recent_10m)} in 10 min`;

const TILE_LABEL_MAX = 16;

const GRAPHEMES = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

/** A Preset tile's name as a Tile label: sentence case and at most 16 characters, never cutting one in half. */
export function presetLabel(name: string): string {
  let label = '';
  for (const { segment } of GRAPHEMES.segment(sentence(name.trim()))) {
    if ([...label].length + [...segment].length > TILE_LABEL_MAX) break;
    label += segment;
  }
  return label.trim() || 'Tile';
}

const UNIT_WORDS = new Set(['pct', 'bytes', 's']);

/** An id in words, without the unit parts the value shows anyway: "network_send_bytes_s" is "Network send". */
export function idWords(id: string): string {
  const words = id.split(/[_\s-]+/).filter(Boolean);
  while (words.length > 1) {
    const last = words.at(-1);
    if (last === undefined || !UNIT_WORDS.has(last)) break;
    words.pop();
  }
  return words.length ? sentence(words.join(' ')) : id;
}

/** A stream's name for people: the name of the Preset tile that shows it, else its id in words. */
export function streamLabel(integration: Pick<IntegrationSummary, 'id' | 'presets'>, stream: string): string {
  const preset = integration.presets.find((p) => {
    const action = parseAction(p.action);
    return action?.type === 'widget' && action.source === integration.id && action.field === stream;
  });
  return preset ? presetLabel(preset.name) : idWords(stream);
}

/** The Preset's action as a Layout Tile action, or null when the Layout rules refuse it. */
export function parseAction(value: JsonValue): Action | null {
  const probe = {
    schema: 1,
    name: 'Preset',
    orientation: 'landscape',
    pages: [{ kind: 'grid', id: 'p', title: 'P', rows: 1, cols: 1, tiles: [{ id: 't', label: 'T', row: 0, col: 0, rowSpan: 1, colSpan: 1, icon: null, action: value }] }],
  };
  const parsed = parseLayout(JSON.stringify(probe));
  if (!parsed.ok) return null;
  const [page] = parsed.layout.pages;
  return page.kind === 'grid' ? page.tiles[0].action : null;
}

/** How a Preset tile is drawn: a bar for a share, a graph for a count, its value as text, or its renderer's look. */
export type PresetLook = 'bar' | 'graph' | 'text' | 'level' | 'now-playing';

export function presetLook(preset: PresetTile, streams: readonly DataStream[]): PresetLook {
  if (preset.render === 'bar') return 'level';
  if (preset.render === 'now-playing') return 'now-playing';
  const action = parseAction(preset.action);
  const stream = action?.type === 'widget' ? streams.find((s) => s.id === action.field) : undefined;
  if (stream?.unit === '%') return 'bar';
  return stream?.kind === 'numeric' ? 'graph' : 'text';
}
