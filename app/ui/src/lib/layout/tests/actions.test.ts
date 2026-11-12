import { describe, expect, test } from 'vitest';
import { ACTION_GROUPS, commandLine, escapeInvisible, freshAction, needsApproval, processOf, sameProcess, spellShortcut, splitArgs, switchKind } from '../actions.ts';
import { friendlyRefusal } from '../messages.ts';
import { setAction, starterLayout, type Action } from '../model.ts';

const PAGES = ['media', 'shortcuts'];

describe('the When tapped kinds', () => {
  test('each kind on offer starts with an action of that kind', () => {
    for (const { type } of ACTION_GROUPS.flatMap((group) => group.kinds)) expect(freshAction(type, PAGES).type).toBe(type);
  });

  test('a Page jump starts on the first other Page', () => {
    expect(freshAction('page', PAGES, 'media')).toEqual({ type: 'page', page: 'shortcuts' });
    expect(freshAction('page', ['media'], 'media')).toEqual({ type: 'page', page: 'media' });
  });
});

describe('changing the kind', () => {
  const launch: Action = { type: 'launch', path: 'notepad.exe', args: ['notes.txt'], cwd: 'C:\\Notes' };

  test('an app becomes a command with its path as the program, its arguments and its folder', () => {
    expect(switchKind(launch, 'command', PAGES)).toEqual({ type: 'command', program: 'notepad.exe', args: ['notes.txt'], cwd: 'C:\\Notes', shell: false });
  });

  test('a command becomes an app and drops the shell', () => {
    const command: Action = { type: 'command', program: 'robocopy', args: ['a', 'b'], cwd: '', shell: true };
    expect(switchKind(command, 'launch', PAGES)).toEqual({ type: 'launch', path: 'robocopy', args: ['a', 'b'], cwd: '' });
  });

  test('kinds that share nothing start fresh, and the same kind keeps the action', () => {
    expect(switchKind(launch, 'open', PAGES)).toEqual({ type: 'open', target: '' });
    expect(switchKind({ type: 'key', usage: 6, mods: ['ctrl'] }, 'consumer', PAGES)).toEqual({ type: 'consumer', usage: 205 });
    expect(switchKind(launch, 'launch', PAGES)).toBe(launch);
  });
});

describe('the form rules', () => {
  function check(action: Action): string | null {
    const layout = starterLayout('landscape');
    const edit = setAction(layout, 'shortcuts', 'copy', action);
    return edit.ok ? null : friendlyRefusal(edit.reason, layout, 'Desk', edit.path);
  }

  test.each<[string, Action, string]>([
    ['an app with no path', { type: 'launch', path: '', args: [], cwd: '' }, 'Choose an app.'],
    ['an open with no target', { type: 'open', target: '' }, 'Enter a file, folder or web address.'],
    ['a command with no program', { type: 'command', program: '', args: [], cwd: '', shell: false }, 'Enter a program.'],
    ['a program with a tab in it', { type: 'command', program: 'a\tb', args: [], cwd: '', shell: false }, 'Remove tabs, line breaks and other special characters.'],
    ['a key code past the list', { type: 'key', usage: 0, mods: [] }, 'Pick a key from the list.'],
    ['a media code past the list', { type: 'consumer', usage: 70000 }, 'Pick a key from the list.'],
    ['a jump to a Page that is gone', { type: 'page', page: 'gone' }, 'A Tile goes to a Page that no longer exists.'],
  ])('refuses %s', (_, action, message) => {
    expect(check(action)).toBe(message);
  });

  test('takes every kind once its fields are filled', () => {
    const filled: Action[] = [
      { type: 'key', usage: 6, mods: ['ctrl', 'shift'] },
      { type: 'consumer', usage: 226 },
      { type: 'mod', mod: 'win' },
      { type: 'launch', path: 'notepad.exe', args: [], cwd: '' },
      { type: 'open', target: 'https://example.com' },
      { type: 'command', program: 'robocopy', args: ['C:\\Projects', 'D:\\Backup'], cwd: '', shell: true },
      { type: 'page', page: 'media' },
      { type: 'none' },
    ];
    for (const action of filled) expect(check(action)).toBeNull();
  });
});

describe('what the form shows', () => {
  test('a shortcut spells its modifiers in a fixed order before the key', () => {
    expect(spellShortcut({ type: 'key', usage: 6, mods: ['shift', 'ctrl'] })).toBe('Ctrl + Shift + C');
    expect(spellShortcut({ type: 'key', usage: 7, mods: ['win'] })).toBe('Windows + D');
    expect(spellShortcut({ type: 'key', usage: 200, mods: [] })).toBe('Key code 200');
  });

  test('arguments are one per line, and an empty box is none', () => {
    expect(splitArgs('')).toEqual([]);
    expect(splitArgs('C:\\Projects\n/MIR')).toEqual(['C:\\Projects', '/MIR']);
  });

  test('a command line quotes the parts that hold spaces or quotes', () => {
    expect(commandLine('robocopy', ['C:\\Projects', 'D:\\My Backup', '/MIR'])).toBe('robocopy C:\\Projects "D:\\My Backup" /MIR');
    expect(commandLine('C:\\Program Files\\Tool.exe', ['say "hi"', ''])).toBe('"C:\\Program Files\\Tool.exe" "say \\"hi\\"" ""');
  });

  test('display text escapes control, format and separator characters as the dialog on the PC does', () => {
    expect(escapeInvisible('a\u202Eb')).toBe('a\\u{202E}b');
    expect(escapeInvisible('\u200Bzero')).toBe('\\u{200B}zero');
    expect(escapeInvisible('\uFEFF')).toBe('\\u{FEFF}');
    expect(escapeInvisible('one\ntwo')).toBe('one\\u{000A}two');
    expect(escapeInvisible('line\u2028end')).toBe('line\\u{2028}end');
    expect(escapeInvisible('Backup to D: now')).toBe('Backup to D: now');
    expect(escapeInvisible('C:\\Program Files\\App\\app.exe')).toBe('C:\\Program Files\\App\\app.exe');
  });
});

describe('asking on this PC first', () => {
  test('commands, apps and opening anything but a web or mail address ask first (ADR 0006)', () => {
    expect(needsApproval({ type: 'command', program: 'robocopy', args: [], cwd: '', shell: false })).toBe(true);
    expect(needsApproval({ type: 'launch', path: 'notepad.exe', args: [], cwd: '' })).toBe(true);
    expect(needsApproval({ type: 'open', target: 'C:\\Projects' })).toBe(true);
    expect(needsApproval({ type: 'open', target: 'https://example.com' })).toBe(false);
    expect(needsApproval({ type: 'open', target: 'MAILTO:someone' })).toBe(false);
    expect(needsApproval({ type: 'key', usage: 6, mods: ['ctrl'] })).toBe(false);
  });

  test('an allowed action matches only the same program, arguments, folder and shell', () => {
    const command = processOf({ type: 'command', program: 'robocopy', args: ['a', 'b'], cwd: '', shell: false })!;
    expect(sameProcess(command, { ...command, args: ['a', 'b'] })).toBe(true);
    expect(sameProcess(command, { ...command, args: ['a'] })).toBe(false);
    expect(sameProcess(command, { ...command, shell: true })).toBe(false);
    expect(sameProcess(command, { ...command, kind: 'launch' })).toBe(false);
  });
});

test('Bluetooth destination follows advertised sinks, including absent diagnostics', async () => {
  const { offersBluetooth } = await import('../actions.ts');
  expect(offersBluetooth(undefined)).toBe(false);
  expect(offersBluetooth({ diagnostics: null })).toBe(false);
  expect(offersBluetooth({ diagnostics: { sinks: ['log', 'link'] } })).toBe(false);
  expect(offersBluetooth({ diagnostics: { sinks: ['log', 'link', 'bluetooth'] } })).toBe(true);
});
