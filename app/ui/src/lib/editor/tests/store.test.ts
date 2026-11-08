import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import { createClient, type Companion } from '../../companion/client.ts';
import { createDevicesStore } from '../../companion/devices.ts';
import { ApiError } from '../../companion/errors.ts';
import { openEvents, type Events } from '../../companion/events.ts';
import { createFakeCompanion, type FakeCompanion } from '../../companion/fake.ts';
import { createJobsStore, waitForJob, type JobsStore } from '../../companion/jobs.ts';
import { sampleDevice } from '../../companion/samples.ts';
import { windowSession } from '../../companion/session.ts';
import { draftKey, type DraftStorage } from '../store.ts';
import { createEditorStore, effectiveTileEdit, firstProblem, presetTile, refusedActionField, saveState, tileProblems, type EditorDeps, type EditorStore } from '../store.ts';
import { switchKind } from '../../layout/actions.ts';
import { canonical, starterLayout, type GridPage, type Layout, type Page } from '../../layout/model.ts';

const BASE = 'http://127.0.0.1:51234';
const DESK = { id: 'mock0001', name: 'Desk' };
const connected = { state: 'connected' as const, port: 'COM3', since_ms: 0, protocol: 2 };

function memoryStorage(): DraftStorage & { items: Map<string, string> } {
  const items = new Map<string, string>();
  return {
    items,
    getItem: (key) => items.get(key) ?? null,
    setItem: (key, value) => void items.set(key, value),
    removeItem: (key) => void items.delete(key),
  };
}

let fake: FakeCompanion;
let companion: Companion;
let jobs: JobsStore;
let storage: ReturnType<typeof memoryStorage>;

function editor(overrides: Partial<EditorDeps> = {}): EditorStore {
  return createEditorStore({ companion, storage, wait: (job) => waitForJob(jobs, companion, job, { pollMs: 10 }), ...overrides });
}

const state = (store: EditorStore) => store.getState();
const draft = (store: EditorStore) => state(store).drafts[state(store).orientation]!.layout;
function gridPage(page: Page | undefined): GridPage {
  if (page?.kind !== 'grid') throw new Error(`${page?.id} is not a grid page`);
  return page;
}
const page = (store: EditorStore) => gridPage(draft(store).pages.find((p) => p.id === state(store).page));
const tile = (store: EditorStore, id: string) => page(store).tiles.find((t) => t.id === id)!;

beforeEach(() => {
  fake = createFakeCompanion({
    devices: [sampleDevice('mock0001', 'Desk', connected), sampleDevice('mock0002', 'Studio', connected)],
    layouts: { mock0001: starterLayout('landscape') },
    jobMs: 5,
  });
  vi.stubGlobal('fetch', fake.fetch);
  companion = createClient(BASE, windowSession(fake.bridge));
  jobs = createJobsStore();
  storage = memoryStorage();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('editing a draft', () => {
  let store: EditorStore;

  beforeEach(() => {
    store = editor();
    store.open(DESK);
    store.startStarter();
  });

  test('a Starter Layout opens on its first Page with nothing selected', () => {
    expect(draft(store).name).toBe('Starter');
    expect(state(store).page).toBe('media');
    expect(state(store).selected).toBeNull();
    expect(state(store).status?.text).toBe('Started the Starter Layout.');
  });

  test('a Tile moves onto a free cell, and a move onto a taken cell is refused with the Tile kept', () => {
    store.setGrid(3, 3);
    expect(store.moveTile('prev', 2, 2)).toBeNull();
    expect(tile(store, 'prev')).toMatchObject({ row: 2, col: 2 });

    expect(store.moveTile('prev', 0, 1)).toBe('overlaps playpause');
    expect(state(store).status).toMatchObject({ kind: 'error', text: 'That spot is taken by “Play/Pause”. Pick an empty one.', event: 'refused' });
    expect(tile(store, 'prev')).toMatchObject({ row: 2, col: 2 });
  });

  test('a Tile grows to 2 × 2, and a size past the grid edge is refused', () => {
    store.setGrid(3, 3);
    store.clearTile('playpause');
    store.clearTile('mute');
    store.clearTile('voldown');
    expect(store.resizeTile('prev', 2, 2)).toBeNull();
    expect(tile(store, 'prev')).toMatchObject({ rowSpan: 2, colSpan: 2 });

    expect(store.resizeTile('prev', 2, 4)).toBe('outside the 3x3 grid');
    expect(state(store).status?.text).toBe("That doesn't fit. This Page has 3 rows and 3 columns.");
    expect(tile(store, 'prev')).toMatchObject({ rowSpan: 2, colSpan: 2 });
  });

  test('Add Tile targets a clicked cell, places the kind’s default action there and selects it', () => {
    store.setGrid(3, 3);
    store.openAdd({ row: 2, col: 1 });
    expect(state(store).adding).toEqual({ row: 2, col: 1 });
    store.place('consumer');
    const added = state(store).selected!;
    expect(tile(store, added)).toMatchObject({ row: 2, col: 1, label: 'Tile', action: { type: 'consumer', usage: 205 } });
    expect(state(store).adding).toBeNull();
  });

  test('Add Tile with no cell picks the first free one', () => {
    store.setGrid(3, 3);
    store.openAdd();
    expect(state(store).adding).toEqual({ row: 2, col: 0 });
  });

  test('a kind that needs a value first starts as Nothing, with its empty form kept on the Tile as invalid', () => {
    store.setGrid(3, 3);
    store.openAdd();
    store.place('launch');
    const added = state(store).selected!;
    expect(tile(store, added).action).toEqual({ type: 'none' });
    expect(tileProblems(state(store), added)?.action).toMatchObject({ value: { type: 'launch', path: '' }, reason: expect.stringMatching(/^text\.empty: .*\/action\/path /) });
  });

  test('Add Tile on a full Page starts a new Page with the Tile on it', () => {
    store.openAdd();
    expect(state(store).adding).toBe('new-page');
    store.place('key');
    expect(draft(store).pages).toHaveLength(3);
    expect(state(store).page).toBe('page1');
    expect(page(store).tiles).toEqual([expect.objectContaining({ row: 0, col: 0, action: { type: 'key', usage: 58, mods: [] } })]);
  });

  test('Duplicate copies a Tile into the next free cell and selects the copy', () => {
    store.setGrid(3, 3);
    store.select('next');
    store.duplicate('next');
    const copy = state(store).selected!;
    expect(copy).not.toBe('next');
    expect(tile(store, copy)).toMatchObject({ row: 2, col: 0, label: 'Next', action: { type: 'consumer', usage: 181 } });
  });

  test('Duplicate on a full Page is refused', () => {
    store.duplicate('next');
    expect(state(store).status?.text).toBe('This Page is full. Make its grid bigger or add a Page.');
    expect(page(store).tiles).toHaveLength(6);
  });

  test('Clear Tile removes it and drops the selection', () => {
    store.select('mute');
    store.clearTile('mute');
    expect(page(store).tiles.map((t) => t.id)).not.toContain('mute');
    expect(state(store).selected).toBeNull();
  });

  test('Pages: add, rename, refuse a blank name and a grid that cuts a Tile, remove', () => {
    store.addPage();
    expect(state(store).page).toBe('page1');
    expect(page(store).title).toBe('Page 3');

    expect(store.renamePage('page1', 'Games')).toBeNull();
    expect(page(store).title).toBe('Games');
    expect(store.renamePage('page1', '')).not.toBeNull();
    expect(state(store).status?.text).toBe('Give the Page a name.');

    store.selectPage('media');
    expect(store.setGrid(1, 3)).toBe('grid.too-small: mute');
    expect(state(store).status?.text).toBe('The grid can\'t shrink past “Mute”. Move or remove it first.');

    store.removePage('page1');
    expect(draft(store).pages.map((p) => p.id)).toEqual(['media', 'shortcuts']);
  });

  test('the last Page cannot be removed', () => {
    store.startBlank();
    store.removePage('page1');
    expect(state(store).status?.text).toBe('A Layout needs at least one Page.');
  });

  test('a file that isn’t a Layout is explained and changes nothing', () => {
    const before = draft(store);
    expect(store.importText('{"schema": 1, "name": "Broken"', 'broken.json')).toBe(false);
    expect(state(store).status).toMatchObject({ kind: 'error', text: '“broken.json” isn\'t a Layout file. Nothing changed.', event: 'open-failed' });
    expect(draft(store)).toBe(before);
  });

  test('an imported file opens its own orientation with a tab per Page', () => {
    const portrait: Layout = { ...starterLayout('portrait'), name: 'Tall' };
    expect(store.importText(JSON.stringify(portrait), 'tall.json')).toBe(true);
    expect(state(store).orientation).toBe('portrait');
    expect(state(store).status?.text).toBe('Imported “Tall”: 2 Pages, 16 Tiles.');
    expect(state(store).drafts.landscape?.layout.name).toBe('Starter');
  });
});

describe('Preset tiles', () => {
  const preset = { id: 'five_hour_pct', name: 'five hour pct', action: { type: 'widget', source: 'cliproxyapi', field: 'five_hour_pct', account: '' } };
  const cpu = { id: 'cpu_pct', name: 'CPU usage', action: { type: 'widget', source: 'this_pc', field: 'cpu_pct', account: '' }, render: 'graph' as const };
  const graph = { render: 'graph', streams: { value: 'number' } } as const;

  test('a Preset whose action breaks the Layout rules is refused', () => {
    const store = editor();
    store.open(DESK);
    store.startBlank();
    store.openAdd({ row: 0, col: 0 });
    store.placePreset({ ...preset, action: { type: 'widget', source: '', field: 'plan', account: '' } });
    expect(state(store).status).toMatchObject({ kind: 'error', text: 'That change breaks a Layout rule. See Details.' });
    expect(page(store).tiles).toEqual([]);
  });

  test('presetTile builds the 1×1 Tile a Preset places, with its name as the label', () => {
    expect(presetTile(preset, null)).toEqual({ id: 'preset', label: 'Five hour pct', row: 0, col: 0, rowSpan: 1, colSpan: 1, icon: null, action: preset.action });
    expect(presetTile(preset, null, { id: 't9', row: 2, col: 3 })).toMatchObject({ id: 't9', row: 2, col: 3 });
  });

  test('presetTile draws the Preset as its look, and refuses an action the Layout rules refuse', () => {
    expect(presetTile(cpu, graph)).toMatchObject({ label: 'CPU usage', render: 'graph', streams: { value: 'number' }, action: cpu.action });
    expect(presetTile({ ...preset, action: { type: 'widget', source: '', field: 'plan', account: '' } }, null)).toBeNull();
    expect(presetTile({ ...preset, action: { type: 'launch-rockets' } }, null)).toBeNull();
  });

  test('placing a Preset from Add Tile puts down exactly the Tile presetTile builds', () => {
    const store = editor();
    store.open(DESK);
    store.startStarter();
    store.setGrid(3, 3);
    store.openAdd({ row: 2, col: 1 });
    store.placePreset(cpu, graph);
    const placed = tile(store, state(store).selected!);
    expect(placed).toEqual(presetTile(cpu, graph, { id: placed.id, row: 2, col: 1 }));
  });

  test('the Add Tile panel places a Preset tile at its target and selects it', () => {
    const store = editor();
    store.open(DESK);
    store.startStarter();
    store.setGrid(3, 3);
    store.openAdd({ row: 2, col: 1 });
    store.placePreset(preset);
    expect(tile(store, state(store).selected!)).toMatchObject({ row: 2, col: 1, label: 'Five hour pct', action: preset.action });
    expect(state(store).adding).toBeNull();
  });


  test('a Preset placed as a graph is a schema 2 graph Tile bound to its stream', () => {
    const store = editor();
    store.open(DESK);
    store.startStarter();
    store.setGrid(3, 3);
    store.openAdd({ row: 2, col: 0 });
    store.placePreset(cpu, graph);
    expect(draft(store).schema).toBe(2);
    expect(tile(store, state(store).selected!)).toMatchObject({ label: 'CPU usage', icon: null, render: 'graph', streams: { value: 'number' }, action: cpu.action });
  });

});

describe('Volume and Media Preset tiles', () => {
  const volumeBar = { id: 'volume_bar', name: 'Volume bar', action: { type: 'widget', source: 'volume', field: 'level', account: '' }, render: 'bar' as const, streams: { value: 'number', muted: 'number' } as const, input: true };
  const bar = { render: 'bar', streams: { value: 'number', muted: 'number' }, input: true } as const;
  const nowPlaying = { render: 'now-playing', streams: { title: 'text', artist: 'text', cover: 'image', state: 'number' } } as const;

  function placedBar() {
    const store = editor();
    store.open(DESK);
    store.startStarter();
    store.setGrid(3, 3);
    store.openAdd({ row: 2, col: 0 });
    store.placePreset(volumeBar, bar);
    return { store, id: state(store).selected! };
  }

  test('a Volume bar placed from Add Tile is a draggable bar Tile in a schema 2 Layout', () => {
    const { store, id } = placedBar();
    expect(draft(store).schema).toBe(2);
    expect(tile(store, id)).toEqual(expect.objectContaining({ label: 'Volume bar', icon: null, render: 'bar', streams: bar.streams, input: true, action: volumeBar.action }));
  });

  test('a Now playing Preset placed from Add Tile is a now-playing Tile that takes no drag', () => {
    const store = editor();
    store.open(DESK);
    store.startStarter();
    store.setGrid(3, 3);
    const preset = { id: 'now_playing', name: 'Now playing', action: { type: 'widget', source: 'media', field: 'title', account: '' }, render: 'now-playing' as const, streams: nowPlaying.streams };
    store.openAdd({ row: 2, col: 0 });
    store.placePreset(preset, nowPlaying);
    expect(tile(store, state(store).selected!)).toMatchObject({ row: 2, col: 0, render: 'now-playing', streams: nowPlaying.streams, action: preset.action });
    expect(tile(store, state(store).selected!)).not.toHaveProperty('input');
  });

  test('a bar Tile can stop taking drags and take them again, and only a bar takes input', () => {
    const { store, id } = placedBar();
    expect(store.setInput(id, false)).toBeNull();
    expect(tile(store, id)).not.toHaveProperty('input');
    expect(store.setInput(id, true)).toBeNull();
    expect(tile(store, id)).toMatchObject({ input: true });
    expect(store.setInput(page(store).tiles[0].id, true)).toMatch(/not a bar/);
  });

  test('a bar Tile can show its plain value or a graph instead, and become the bar again', () => {
    const { store, id } = placedBar();
    const action = { type: 'widget', source: 'volume', field: 'level', account: '' } as const;
    expect(store.setLive(id, action, null)).toBeNull();
    expect(tile(store, id)).toEqual(expect.objectContaining({ icon: null, action }));
    expect(tile(store, id)).not.toHaveProperty('render');
    expect(tile(store, id)).not.toHaveProperty('input');
    expect(store.setGraph(id, { integration: 'volume', stream: 'level' })).toBeNull();
    expect(tile(store, id)).toMatchObject({ render: 'graph' });
    expect(tile(store, id)).not.toHaveProperty('input');
    expect(store.setLive(id, action, bar)).toBeNull();
    expect(tile(store, id)).toMatchObject({ render: 'bar', input: true, streams: bar.streams, action });
  });

  test('a bar Tile that plays a GIF instead takes no drags, and can become the bar again', () => {
    const { store, id } = placedBar();
    const action = { type: 'widget', source: 'volume', field: 'level', account: '' } as const;
    expect(store.setGif(id, 'a'.repeat(64))).toBeNull();
    expect(tile(store, id)).toMatchObject({ render: 'gif', asset: `sha256:${'a'.repeat(64)}`, action });
    expect(tile(store, id)).not.toHaveProperty('input');
    expect(tile(store, id)).not.toHaveProperty('streams');
    expect(store.setLive(id, action, bar)).toBeNull();
    expect(tile(store, id)).toMatchObject({ render: 'bar', input: true, streams: bar.streams, action });
    expect(tile(store, id)).not.toHaveProperty('asset');
  });
});

describe('graph Tiles', () => {
  test('any Tile can draw a graph of a stream, change stream, and go back to its label', () => {
    const store = editor();
    store.open(DESK);
    store.startStarter();
    const first = page(store).tiles[0];
    expect(store.setGraph(first.id, { integration: 'this_pc', stream: 'ram_pct' })).toBeNull();
    expect(draft(store).schema).toBe(2);
    expect(tile(store, first.id)).toMatchObject({
      label: first.label,
      icon: null,
      render: 'graph',
      streams: { value: 'number' },
      action: { type: 'widget', source: 'this_pc', field: 'ram_pct', account: '' },
    });
    store.setGraph(first.id, { integration: 'this_pc', stream: 'cpu_pct' });
    expect(tile(store, first.id).action).toEqual({ type: 'widget', source: 'this_pc', field: 'cpu_pct', account: '' });
    store.setGraph(first.id, null);
    const back = tile(store, first.id);
    expect(back.render).toBeUndefined();
    expect(back.streams).toBeUndefined();
    expect(back.action).toEqual({ type: 'widget', source: 'this_pc', field: 'cpu_pct', account: '' });
  });

  test('a graph Tile survives export and import', () => {
    const store = editor();
    store.open(DESK);
    store.startStarter();
    const first = page(store).tiles[0];
    store.setGraph(first.id, { integration: 'this_pc', stream: 'cpu_pct' });
    const text = store.exportText()!;
    expect(store.importText(text, 'graph.json')).toBe(true);
    expect(tile(store, first.id)).toMatchObject({ render: 'graph', streams: { value: 'number' } });
  });
});

describe('the invalid-Tile rule', () => {
  let store: EditorStore;

  beforeEach(() => {
    store = editor();
    store.open(DESK);
    store.startStarter();
    store.setGrid(3, 3);
  });

  test('the effective edit follows a refusal, a kind switch and repair through sending', async () => {
    const edit = () => effectiveTileEdit(state(store), DESK.name, 'next')!;
    const command = { type: 'command' as const, program: '', args: [], cwd: '', shell: false };
    store.editAction('next', command);
    expect(edit()).toEqual({
      label: { value: 'Next', error: null },
      action: { value: command, error: 'Enter a program.', field: 'program' },
      kind: 'command', unplacedError: null,
    });
    const open = switchKind(edit().action.value, 'open', draft(store).pages.map((p) => p.id), page(store).id);
    store.editAction('next', open);
    expect(edit()).toEqual({
      label: { value: 'Next', error: null },
      action: { value: open, error: 'Enter a file, folder or web address.', field: 'target' },
      kind: 'open', unplacedError: null,
    });
    await store.push();
    expect(fake.layouts.puts).toBe(0);
    const repaired = { type: 'open' as const, target: 'https://example.com' };
    store.editAction('next', repaired);
    expect(edit()).toEqual({
      label: { value: 'Next', error: null },
      action: { value: repaired, error: null, field: null },
      kind: 'open', unplacedError: null,
    });
    await store.push();
    expect(state(store).status?.event).toBe('pushed');
    expect(fake.layouts.puts).toBe(1);
  });

  test('an action the rules refuse stays on the Tile with its reason, and a valid one clears it', () => {
    expect(store.editAction('next', { type: 'command', program: '', args: [], cwd: '', shell: false })).toMatch(/^text\.empty: /);
    expect(tile(store, 'next').action).toEqual({ type: 'consumer', usage: 181 });
    expect(tileProblems(state(store), 'next')?.action?.value).toEqual({ type: 'command', program: '', args: [], cwd: '', shell: false });

    expect(store.editAction('next', { type: 'command', program: 'notepad.exe', args: [], cwd: '', shell: false })).toBeNull();
    expect(tile(store, 'next').action).toEqual({ type: 'command', program: 'notepad.exe', args: [], cwd: '', shell: false });
    expect(tileProblems(state(store), 'next')).toBeUndefined();
  });

  test('a refused action names its field by pointer, whatever its reason says', () => {
    store.editAction('next', { type: 'command', program: '', args: [], cwd: '', shell: false });
    const refused = tileProblems(state(store), 'next')!.action!;
    expect(refused.path).toMatch(/\/action\/program$/);
    expect(refusedActionField(refused)).toBe('program');
    expect(refusedActionField({ ...refused, reason: 'reworded without a pointer' })).toBe('program');
    expect(refusedActionField({ ...refused, path: '/pages/1/tiles/0/action/args/2' })).toBe('args');
    expect(refusedActionField({ ...refused, path: '' })).toBeNull();
  });

  test('a refused label stays on the Tile until a valid one replaces it', () => {
    expect(store.editLabel('next', '   ')).toMatch(/^label\.blank: /);
    expect(tile(store, 'next').label).toBe('Next');
    expect(tileProblems(state(store), 'next')?.label?.value).toBe('   ');
    expect(store.editLabel('next', 'Skip')).toBeNull();
    expect(tile(store, 'next').label).toBe('Skip');
    expect(tileProblems(state(store), 'next')).toBeUndefined();
  });

  test('Send to Device is refused with the rule message and selects the invalid Tile', async () => {
    store.selectPage('shortcuts');
    store.editAction('copy', { type: 'open', target: '' });
    store.selectPage('media');
    await store.push();
    expect(state(store).status).toMatchObject({ kind: 'error', text: 'Enter a file, folder or web address.', event: 'refused' });
    expect(state(store)).toMatchObject({ page: 'shortcuts', selected: 'copy' });
    expect(fake.layouts.puts).toBe(0);

    store.editAction('copy', { type: 'open', target: 'https://example.com' });
    expect(firstProblem(state(store))).toBeNull();
    await store.push();
    expect(state(store).status?.event).toBe('pushed');
  });

  test('Clear Tile drops its problems; Duplicate copies them', () => {
    store.editAction('next', { type: 'launch', path: '', args: [], cwd: '' });
    store.duplicate('next');
    const copy = state(store).selected!;
    expect(tileProblems(state(store), copy)?.action?.value.type).toBe('launch');
    store.clearTile('next');
    store.clearTile(copy);
    expect(state(store).invalid.landscape).toEqual({});
  });

  test('a new Layout starts with no invalid Tiles', () => {
    store.editAction('next', { type: 'launch', path: '', args: [], cwd: '' });
    store.startStarter();
    expect(firstProblem(state(store))).toBeNull();
  });
});

describe('drafts', () => {
  test('each orientation keeps its own draft', () => {
    const store = editor();
    store.open(DESK);
    store.startStarter();
    store.setOrientation('portrait');
    expect(state(store).drafts.portrait).toBeNull();
    store.startBlank();
    expect(draft(store).orientation).toBe('portrait');
    store.setOrientation('landscape');
    expect(draft(store).name).toBe('Starter');
  });

  test('drafts are saved per Device and orientation and come back when the Device opens again', () => {
    const first = editor();
    first.open(DESK);
    first.startStarter();
    first.renameLayout('Mine');
    first.open({ id: 'mock0002', name: 'Studio' });
    expect(state(first).drafts.landscape).toBeNull();
    first.startBlank();

    expect([...storage.items.keys()].sort()).toEqual([draftKey('mock0001', 'landscape'), draftKey('mock0002', 'landscape')]);

    const second = editor();
    second.open(DESK);
    expect(draft(second).name).toBe('Mine');
    second.open({ id: 'mock0002', name: 'Studio' });
    expect(draft(second).name).toBe('New Layout');
  });

  test('a saved draft that no longer passes the rules is dropped', () => {
    storage.setItem(draftKey('mock0001', 'landscape'), JSON.stringify({ v: 1, layout: '{"schema":1}', sent: null, revision: null }));
    const store = editor();
    store.open(DESK);
    expect(state(store).drafts.landscape).toBeNull();
  });
});

describe('logos a Brandfetch refresh replaced', () => {
  const OLD = 'a'.repeat(64);
  const NEW = 'b'.repeat(64);
  const NEWER = 'c'.repeat(64);

  test('a draft that shows the old Asset gets the new one, and a push sends the new one', async () => {
    const store = editor();
    store.open(DESK);
    store.startStarter();
    store.selectPage('shortcuts');
    store.setAsset('copy', OLD);
    await store.push();
    expect(saveState(state(store))).toBe('saved');

    store.replaceAssets({ [OLD]: NEW });
    expect(tile(store, 'copy').asset).toBe(`sha256:${NEW}`);
    expect(saveState(state(store))).toBe('unsent');
    expect(storage.items.get(draftKey('mock0001', 'landscape'))).toContain(NEW);
    await store.push();
    expect(state(store).status?.event).toBe('pushed');
    expect(fake.layouts.stored('mock0001', 'landscape')).toContain(`sha256:${NEW}`);
    expect(fake.layouts.stored('mock0001', 'landscape')).not.toContain(OLD);
  });

  test('a draft read back from storage, and an Asset picked from Recents, get the replacement too', () => {
    const first = editor();
    first.open(DESK);
    first.startStarter();
    first.selectPage('shortcuts');
    first.setAsset('copy', OLD);

    const second = editor();
    second.replaceAssets({ [OLD]: NEWER });
    second.open(DESK);
    second.selectPage('shortcuts');
    expect(tile(second, 'copy').asset).toBe(`sha256:${NEWER}`);
    second.setAsset('copy', OLD);
    expect(tile(second, 'copy').asset).toBe(`sha256:${NEWER}`);
  });

  test('a draft with no replaced Asset stays as it is', () => {
    const store = editor();
    store.open(DESK);
    store.startStarter();
    const before = state(store).drafts.landscape;
    store.replaceAssets({ [OLD]: NEW });
    expect(state(store).drafts.landscape).toBe(before);
  });
});

describe('send and load', () => {
  test('Send shows Sending…, then the result with the Device’s counts, and the draft reads as saved', async () => {
    const store = editor();
    store.open(DESK);
    store.startStarter();
    expect(saveState(state(store))).toBe('unsent');
    const sending = store.push();
    expect(state(store).status).toMatchObject({ text: 'Sending…', event: 'pushing' });
    expect(saveState(state(store))).toBe('sending');
    await sending;
    expect(state(store).status).toMatchObject({ kind: 'ok', text: 'Sent to Desk', event: 'pushed', pages: 2, tiles: 16 });
    expect(state(store).status?.detail).toMatch(/^Desk shows Starter: 2 Pages, 16 Tiles \(\d+ bytes in \d+ ms\), 0 icons sent$/);
    expect(saveState(state(store))).toBe('saved');
    expect(fake.layouts.stored('mock0001', 'landscape')).toBe(canonical(draft(store)));

    store.renameLayout('Changed');
    expect(saveState(state(store))).toBe('unsent');
  });

  test('a Job follows job.updated from the event stream', async () => {
    const devices = createDevicesStore();
    const events: Events = openEvents({ url: 'ws://127.0.0.1:51234/api/v1/events', session: windowSession(fake.bridge), store: devices, jobs, open: fake.openSocket });
    await vi.waitFor(() => expect(devices.getState().status).toBe('live'));
    const store = editor({ wait: (job) => waitForJob(jobs, companion, job, { pollMs: 60_000 }) });
    store.open(DESK);
    store.startStarter();
    await store.push();
    expect(state(store).status?.event).toBe('pushed');
    expect(Object.values(jobs.getState().jobs).map((j) => j.state)).toEqual(['succeeded']);
    events.close();
  });

  test('Load from the Device with no draft loads at once', async () => {
    const store = editor();
    store.open(DESK);
    await store.load();
    expect(state(store).status).toMatchObject({ kind: 'ok', text: 'Loaded “Starter” from Desk', event: 'pulled' });
    expect(saveState(state(store))).toBe('saved');
  });

  test('Load from the Device asks before it replaces a draft that differs', async () => {
    const store = editor();
    store.open(DESK);
    store.startBlank();
    await store.load();
    expect(state(store).conflict).toMatchObject({ reason: 'load', layout: { name: 'Starter' } });
    expect(state(store).status?.text).toBe('Desk has “Starter”: 2 Pages, 16 Tiles.');
    expect(draft(store).name).toBe('New Layout');

    store.replace();
    expect(draft(store).name).toBe('Starter');
    expect(state(store).conflict).toBeNull();
    expect(canonical(draft(store))).toBe(fake.layouts.stored('mock0001', 'landscape'));
  });

  test('Keep mine keeps the draft, and the next send replaces what the Device has', async () => {
    const store = editor();
    store.open(DESK);
    store.startBlank();
    await store.load();
    store.keepMine();
    expect(state(store).status?.text).toBe('Kept your Layout');
    expect(draft(store).name).toBe('New Layout');
    await store.push();
    expect(state(store).status?.event).toBe('pushed');
    expect(JSON.parse(fake.layouts.stored('mock0001', 'landscape')!).name).toBe('New Layout');
  });

  test('a Device with no Layout for the orientation says so', async () => {
    const store = editor();
    store.open(DESK);
    store.setOrientation('portrait');
    await store.load();
    expect(state(store).status).toMatchObject({ kind: 'error', text: 'Desk has no portrait Layout yet.', event: 'pull-failed' });
  });

  test('412: a Layout changed on the Device opens Replace or Keep mine', async () => {
    const store = editor();
    store.open(DESK);
    await store.load();
    store.renameLayout('Mine');
    fake.layouts.store('mock0001', { ...starterLayout('landscape'), name: 'Elsewhere' });

    await store.push();
    expect(state(store).conflict).toMatchObject({ reason: 'changed', layout: { name: 'Elsewhere' } });
    expect(state(store).status).toMatchObject({ text: 'Desk has “Elsewhere”: 2 Pages, 16 Tiles.', event: 'changed' });
    expect(JSON.parse(fake.layouts.stored('mock0001', 'landscape')!).name).toBe('Elsewhere');

    store.keepMine();
    await store.push();
    expect(state(store).status?.event).toBe('pushed');
    expect(JSON.parse(fake.layouts.stored('mock0001', 'landscape')!).name).toBe('Mine');
  });

  test('412 then Replace makes the Device’s Layout the draft', async () => {
    const store = editor();
    store.open(DESK);
    await store.load();
    store.renameLayout('Mine');
    fake.layouts.store('mock0001', { ...starterLayout('landscape'), name: 'Elsewhere' });
    await store.push();
    store.replace();
    expect(draft(store).name).toBe('Elsewhere');
    expect(saveState(state(store))).toBe('saved');
  });

  test('422: the Companion’s issues become the rule message', async () => {
    const invalid = { code: 'layout.invalid', message: 'The Layout has 1 problems.', details: { issues: [{ rule: 'label.too-long', path: '/pages/0/tiles/0/label', message: '/pages/0/tiles/0/label must NOT have more than 16 characters (maxLength)' }] } };
    const store = editor({
      companion: { ...companion, layouts: { ...companion.layouts, push: () => Promise.reject(new ApiError(invalid.code, invalid.message, invalid.details)) } },
    });
    store.open(DESK);
    store.startStarter();
    await store.push();
    expect(state(store).status).toMatchObject({ kind: 'error', text: 'Tile labels can be up to 16 characters.', event: 'push-failed' });
    expect(state(store).status?.detail).toContain('label.too-long');
    expect(state(store).busy).toBeNull();
  });

  test('a busy Device says so', async () => {
    fake.layouts.hold('mock0001');
    const first = editor();
    first.open(DESK);
    first.startStarter();
    const holding = first.push();
    await vi.waitFor(() => expect(Object.keys(fake.layouts.jobs())).toHaveLength(1));

    const second = editor();
    second.open(DESK);
    second.startBlank();
    await second.push();
    expect(state(second).status).toMatchObject({ kind: 'error', text: 'Desk is busy. Wait a moment, then retry.' });

    fake.layouts.release('mock0001');
    await holding;
    expect(state(first).status?.event).toBe('pushed');
  });

  test('a Device that isn’t connected can’t take a send', async () => {
    fake.update('mock0001', { link: { state: 'lost', reason: 'unplugged', detail: '', at_ms: 0 } });
    const store = editor();
    store.open(DESK);
    store.startStarter();
    await store.push();
    expect(state(store).status?.text).toBe("Couldn't send to Desk. Check that it's plugged in, then retry.");
  });

  test('Remove unused Assets keeps the Assets the drafts use', async () => {
    const prune = vi.fn(companion.assets.prune);
    const store = editor({ companion: { ...companion, assets: { ...companion.assets, prune } } });
    store.open(DESK);
    const layout = starterLayout('landscape');
    gridPage(layout.pages[0]).tiles[0].icon = 'file:0123456789abcdef';
    store.importText(JSON.stringify(layout), 'icons.json');
    await store.pruneAssets();
    expect(prune).toHaveBeenCalledWith({ device: 'mock0001' }, null, { also_keep: ['0123456789abcdef'] });
    expect(state(store).status).toMatchObject({ kind: 'ok', text: 'Removed 0 unused Assets from Desk' });
  });
});

describe('a push refused for its GIFs', () => {
  const refusedBy = (code: string, details: unknown) => {
    const store = editor({ wait: () => Promise.reject(new ApiError(code, 'raw message', details)) });
    store.open(DESK);
    store.startStarter();
    return store;
  };

  test.each([
    ['layout.animation_limit', { page: 'media', field: 'page_tiles', limit: 4, actual: 5 }, 'This Page has too many animations for Desk. Remove an animated Tile.'],
    ['asset.too_large', { reason: 'size', field: 'device_limit', limit: null, actual: null, unit: 'unknown' }, 'This GIF is too large for Desk. Choose a smaller GIF.'],
    ['asset.too_large', { field: 'gif_frames', limit: 60, actual: 61, unit: 'frames' }, 'This GIF is too long for Desk. Choose a shorter GIF.'],
    ['asset.unsupported', { field: 'format', limit: 'supported encoding', actual: 'refused', reason: 'format' }, "Desk can't play this GIF. Update its Firmware, then retry."],
    ['asset.refused', { reason: 'memory' }, 'This GIF is too large for Desk. Choose a smaller GIF.'],
    ['asset.refused', { reason: 'full' }, 'Desk is out of space. Remove unused Assets, then retry.'],
  ])('%s says what to change, with the raw reason under Details', async (code, details, text) => {
    const store = refusedBy(code, details);
    await store.push();
    expect(state(store).status).toMatchObject({ kind: 'error', text, event: 'push-failed' });
    expect(state(store).status?.detail).toContain(code);
  });

  test('a refusal that is not about a GIF keeps the plug-in sentence', async () => {
    const store = refusedBy('asset.refused', { reason: 'storage' });
    await store.push();
    expect(state(store).status?.text).toBe("Couldn't send to Desk. Check that it's plugged in, then retry.");
  });
});
