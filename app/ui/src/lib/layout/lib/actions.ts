import { MODIFIERS, type Action, type ActionType, type Modifier } from './validate.ts';

export interface Choice {
  usage: number;
  name: string;
}

export const KEYS: readonly Choice[] = [
  ...Array.from({ length: 26 }, (_, i) => ({ usage: 4 + i, name: String.fromCharCode(65 + i) })),
  ...Array.from({ length: 9 }, (_, i) => ({ usage: 30 + i, name: String(i + 1) })),
  { usage: 39, name: '0' },
  ...Array.from({ length: 12 }, (_, i) => ({ usage: 58 + i, name: `F${i + 1}` })),
  ...Array.from({ length: 12 }, (_, i) => ({ usage: 104 + i, name: `F${i + 13}` })),
];

export const MEDIA: readonly Choice[] = [
  { usage: 205, name: 'Play or pause' },
  { usage: 181, name: 'Next track' },
  { usage: 182, name: 'Previous track' },
  { usage: 183, name: 'Stop' },
  { usage: 226, name: 'Mute' },
  { usage: 233, name: 'Volume up' },
  { usage: 234, name: 'Volume down' },
];

export const MOD_NAMES: Record<Modifier, string> = { ctrl: 'Ctrl', shift: 'Shift', alt: 'Alt', win: 'Windows key' };

export const ACTION_GROUPS: readonly { name: string; kinds: readonly { type: ActionType; name: string }[] }[] = [
  {
    name: 'Keyboard',
    kinds: [
      { type: 'key', name: 'Press a shortcut' },
      { type: 'consumer', name: 'Media key' },
      { type: 'mod', name: 'Hold Ctrl, Shift, Alt or Windows' },
    ],
  },
  {
    name: 'This PC',
    kinds: [
      { type: 'launch', name: 'Open an app' },
      { type: 'open', name: 'Open a file, folder or website' },
      { type: 'command', name: 'Run a command' },
    ],
  },
  {
    name: 'Device',
    kinds: [
      { type: 'page', name: 'Go to a Page' },
      { type: 'none', name: 'Nothing' },
    ],
  },
];

export const PROXY_FIELDS: Readonly<Record<string, string>> = {
  five_hour_pct: '5-hour used',
  seven_day_pct: '7-day used',
  five_hour_reset: '5-hour reset in',
  seven_day_reset: '7-day reset in',
  plan: 'Plan',
  status: 'Account status',
  success: 'Requests OK',
  failed: 'Requests failed',
  recent_10m: 'Requests 10 min',
};

export function freshAction(type: ActionType, pages: readonly string[], here?: string): Action {
  switch (type) {
    case 'key':
      return { type, usage: 58, mods: [] };
    case 'mod':
      return { type, mod: 'ctrl' };
    case 'consumer':
      return { type, usage: 205 };
    case 'page':
      return { type, page: pages.find((page) => page !== here) ?? pages[0] ?? '' };
    case 'launch':
      return { type, path: '', args: [], cwd: '' };
    case 'open':
      return { type, target: '' };
    case 'command':
      return { type, program: '', args: [], cwd: '', shell: false };
    case 'widget':
      return { type, source: 'cliproxyapi', field: 'five_hour_pct', account: '' };
    case 'none':
      return { type };
    default: {
      const unknown: never = type;
      return unknown;
    }
  }
}

function started(action: Action): { program: string; args: string[]; cwd: string } | null {
  if (action.type === 'launch') return { program: action.path, args: action.args, cwd: action.cwd };
  if (action.type === 'command') return { program: action.program, args: action.args, cwd: action.cwd };
  return null;
}

export function switchKind(from: Action, to: ActionType, pages: readonly string[], here?: string): Action {
  if (from.type === to) return from;
  const fresh = freshAction(to, pages, here);
  const kept = started(from);
  if (!kept) return fresh;
  if (fresh.type === 'launch') return { ...fresh, path: kept.program, args: kept.args, cwd: kept.cwd };
  if (fresh.type === 'command') return { ...fresh, program: kept.program, args: kept.args, cwd: kept.cwd };
  return fresh;
}

export function spellShortcut(action: Extract<Action, { type: 'key' }>): string {
  const key = KEYS.find((k) => k.usage === action.usage)?.name ?? `Key code ${action.usage}`;
  return [...MODIFIERS.filter((m) => action.mods.includes(m)).map((m) => MOD_NAMES[m].replace(' key', '')), key].join(' + ');
}

export const splitArgs = (text: string) => (text === '' ? [] : text.split('\n'));

export const isWebAddress = (target: string) => /^(?:https?|mailto):/i.test(target.trim());

export interface Started {
  kind: 'command' | 'launch' | 'open';
  program: string;
  args: string[];
  cwd: string;
  shell: boolean;
}

export function processOf(action: Action): Started | null {
  switch (action.type) {
    case 'command':
      return { kind: 'command', program: action.program, args: action.args, cwd: action.cwd, shell: action.shell };
    case 'launch':
      return { kind: 'launch', program: action.path, args: action.args, cwd: action.cwd, shell: false };
    case 'open':
      return { kind: 'open', program: action.target, args: [], cwd: '', shell: false };
    default:
      return null;
  }
}

export function needsApproval(action: Action): boolean {
  return action.type === 'command' || action.type === 'launch' || (action.type === 'open' && !isWebAddress(action.target));
}

type Process = Pick<Started, 'program' | 'args' | 'cwd' | 'shell'> & { kind: string };

export function sameProcess(a: Process, b: Process): boolean {
  return a.kind === b.kind && a.program === b.program && a.cwd === b.cwd && a.shell === b.shell && a.args.length === b.args.length && a.args.every((arg, i) => arg === b.args[i]);
}

export function commandLine(program: string, args: readonly string[]): string {
  const quote = (part: string) => (part === '' || /[\s"]/.test(part) ? `"${part.replaceAll('"', '\\"')}"` : part);
  return [quote(program), ...args.map(quote)].join(' ');
}

const INVISIBLE = /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/gu;

export const escapeInvisible = (text: string) =>
  text.replace(INVISIBLE, (char) => `\\u{${(char.codePointAt(0) ?? 0).toString(16).toUpperCase().padStart(4, '0')}}`);

/** Whether the Device advertises the Bluetooth destination, or null while its sinks are unknown (not connected yet). */
export function offersBluetooth(device: { diagnostics?: { sinks: readonly string[] } | null } | undefined): boolean | null {
  const sinks = device?.diagnostics?.sinks;
  return sinks ? sinks.includes('bluetooth') : null;
}
