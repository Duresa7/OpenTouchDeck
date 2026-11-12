import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'vitest';
import {
  addPage,
  addTile,
  canonical,
  freeCell,
  freshId,
  moveTile,
  newGridPage,
  newTile,
  parseLayout,
  removePage,
  removeTile,
  resizeTile,
  setAction,
  setAsset,
  setFullScreen,
  setGraph,
  setGridSize,
  setIcon,
  setLayoutName,
  setPageTitle,
  starterLayout,
  setLabel,
  validateLayout,
  validateSource,
  RENDER_STREAMS,
  type Edit,
  type GridPage,
  type Layout,
  type Page,
  type Tile,
} from '../model.ts';

const schema = join(import.meta.dirname, '..', '..', '..', '..', '..', '..', 'schema');
const examples = join(schema, 'examples');
const invalid = join(examples, 'invalid');

const dev = () => {
  const parsed = parseLayout(readFileSync(join(examples, 'dev.json')));
  assert.ok(parsed.ok);
  return parsed.layout;
};

const tile = (id: string, row: number, col: number): Tile => ({ ...newTile(id, row, col), label: id.toUpperCase() });

const small = (): Layout => ({
  schema: 1,
  name: 'Small',
  orientation: 'landscape',
  pages: [
    { ...newGridPage('main', 'MAIN'), rows: 2, cols: 3, tiles: [tile('a', 0, 0), tile('b', 0, 1)] },
    { ...newGridPage('other', 'OTHER'), rows: 1, cols: 1 },
  ],
});

function gridPage(page: Page | undefined): GridPage {
  if (page?.kind !== 'grid') throw new Error(`${page?.id} is not a grid page`);
  return page;
}
const grid = (layout: Layout, page: string) => gridPage(layout.pages.find((p) => p.id === page));

function ok(edit: Edit): Layout {
  assert.ok(edit.ok, edit.ok ? '' : edit.reason);
  return edit.layout;
}

function refused(edit: Edit): string {
  assert.ok(!edit.ok, 'the edit was accepted');
  return edit.reason;
}

test('the bundled examples are valid', () => {
  for (const name of ['builtin.json', 'dev.json', 'gif.json', 'graph.json', 'media.json']) {
    assert.deepEqual(validateSource(readFileSync(join(examples, name))), [], name);
  }
});

test('each of the invalid examples fails with exactly its rule key, as check-layouts.mjs reports', () => {
  const files = readdirSync(invalid).filter((name) => name.endsWith('.json'));
  assert.equal(files.length, 56);
  const keys = new Set<string>();
  for (const name of files) {
    const bytes = readFileSync(join(invalid, name));
    const end = bytes.indexOf(0x0a);
    const expected = /^expected: ([a-z][a-z0-9.-]*)\r?$/.exec(bytes.subarray(0, end).toString())?.[1];
    assert.ok(expected, `${name} names no rule key`);
    const errors = validateSource(bytes.subarray(end + 1));
    assert.deepEqual(
      errors.map((error) => error.key),
      [expected],
      `${name}: ${JSON.stringify(errors)}`,
    );
    keys.add(expected);
  }
  const readme = readFileSync(join(schema, 'README.md'), 'utf8');
  const documented = new Set(
    readme
      .split('\n')
      .map((line) => /^\| `([a-z0-9.-]+\.[a-z0-9.-]+)`/.exec(line)?.[1])
      .filter((key): key is string => key !== undefined),
  );
  assert.deepEqual([...keys].sort(), [...documented].sort());
});

test('RENDER_STREAMS binds the streams the shared renderer table lists', () => {
  const table: unknown = JSON.parse(readFileSync(join(schema, 'live', 'renderers.json'), 'utf8'));
  assert.ok(table && typeof table === 'object' && 'renderers' in table);
  const renderers = table.renderers;
  assert.ok(renderers && typeof renderers === 'object');
  const shared = Object.fromEntries(Object.entries(renderers).map(([name, value]: [string, unknown]) => {
    assert.ok(value && typeof value === 'object' && 'streams' in value);
    return [name, value.streams];
  }));
  assert.deepEqual(RENDER_STREAMS, shared);
});

test('a bar Tile may leave out muted, and only a bar with a widget action takes input', () => {
  const parsed = parseLayout(readFileSync(join(examples, 'media.json')));
  assert.ok(parsed.ok);
  const media = parsed.layout;
  const level = grid(media, 'media').tiles.find((t) => t.id === 'level');
  assert.ok(level);
  assert.deepEqual(level.streams, { value: 'number' });
  const volume = grid(media, 'media').tiles.find((t) => t.id === 'volume');
  assert.ok(volume);
  assert.equal(volume.input, true);
  assert.deepEqual(validateLayout(media), []);
  const graphed = ok(setGraph(media, 'media', 'volume', { integration: 'this_pc', stream: 'cpu_pct' }));
  const changed = grid(graphed, 'media').tiles.find((t) => t.id === 'volume');
  assert.ok(changed);
  assert.equal(Object.hasOwn(changed, 'input'), false);
});

test('text rules apply to the raw source', () => {
  const text = readFileSync(join(examples, 'builtin.json'), 'utf8');
  assert.deepEqual(validateSource(new Uint8Array([0xef, 0xbb, 0xbf, ...new TextEncoder().encode(text)])).map((e) => e.key), ['file.json']);
  assert.deepEqual(validateSource(text.replace('"rows": 4', '"rows": 4.0')).map((e) => e.key), ['json.integer-form']);
  assert.deepEqual(validateSource(text.replace('"col": 0', '"col": -0')).map((e) => e.key), ['json.integer-form']);
  const nested = (levels: number) => '['.repeat(levels) + ']'.repeat(levels);
  assert.deepEqual(validateSource(nested(16)).map((e) => e.key), ['shape.type']);
  assert.deepEqual(validateSource(nested(17)).map((e) => e.key), ['file.json']);
});

test('the size limit counts the canonical compact form', () => {
  const layout = dev();
  const launch = grid(layout, 'desktop').tiles.find((t) => t.id === 'launch')!;
  assert.equal(launch.action.type, 'launch');
  if (launch.action.type !== 'launch') return;
  launch.action.cwd = '';
  launch.action.cwd = 'a'.repeat(32768 - canonical(layout).length);
  assert.equal(canonical(layout).length, 32768);
  assert.deepEqual(validateLayout(layout), []);
  launch.action.cwd += 'a';
  assert.deepEqual(validateLayout(layout).map((e) => e.key), ['layout.too-large']);
});

test('addTile accepts a free cell and refuses an overlap, the edge, or a duplicate id', () => {
  const layout = small();
  assert.equal(grid(ok(addTile(layout, 'main', tile('c', 1, 2))), 'main').tiles.length, 3);
  assert.equal(refused(addTile(layout, 'main', tile('c', 0, 1))), 'overlaps b');
  assert.equal(refused(addTile(layout, 'main', tile('c', 2, 0))), 'outside the 2x3 grid');
  assert.match(refused(addTile(layout, 'main', tile('a', 1, 0))), /^id\.duplicate: /);
  assert.equal(refused(addTile(layout, 'nope', tile('c', 0, 0))), 'no page nope');
  assert.equal(grid(layout, 'main').tiles.length, 2, 'a refused edit leaves the layout as it was');
});

test('a 65th tile is refused', () => {
  const layout = small();
  layout.pages[1] = { ...newGridPage('other', 'OTHER'), rows: 8, cols: 8, tiles: Array.from({ length: 64 }, (_, i) => tile(`t${i}`, Math.floor(i / 8), i % 8)) };
  assert.deepEqual(validateLayout(layout), []);
  assert.equal(refused(addTile(layout, 'other', tile('t64', 0, 0))), 'a page holds at most 64 tiles');
});

test('moveTile lands on a free cell and names the tile it would cover', () => {
  const layout = small();
  const moved = grid(ok(moveTile(layout, 'main', 'a', 1, 2)), 'main').tiles[0];
  assert.deepEqual([moved.row, moved.col], [1, 2]);
  assert.equal(refused(moveTile(layout, 'main', 'a', 0, 1)), 'overlaps b');
  assert.equal(refused(moveTile(layout, 'main', 'a', 0, 3)), 'outside the 2x3 grid');
  assert.deepEqual(ok(moveTile(layout, 'main', 'a', 0, 0)), layout, 'a tile never overlaps itself');
});

test('resizeTile grows into free cells and stops at the edge or a neighbour', () => {
  const layout = ok(moveTile(small(), 'main', 'b', 0, 2));
  const grown = grid(ok(resizeTile(layout, 'main', 'a', 2, 2)), 'main').tiles[0];
  assert.deepEqual([grown.rowSpan, grown.colSpan], [2, 2]);
  assert.equal(refused(resizeTile(layout, 'main', 'a', 3, 1)), 'outside the 2x3 grid');
  assert.equal(refused(resizeTile(layout, 'main', 'a', 1, 3)), 'overlaps b');
  refused(resizeTile(layout, 'main', 'a', 0, 1));
});

test('setAction keeps a valid action and refuses an invalid one', () => {
  const layout = small();
  const key = { type: 'key', usage: 62, mods: ['ctrl', 'shift'] } as const;
  assert.deepEqual(grid(ok(setAction(layout, 'main', 'a', { ...key, mods: [...key.mods] })), 'main').tiles[0].action, key);
  assert.match(refused(setAction(layout, 'main', 'a', { type: 'key', usage: 0, mods: [] })), /^key\.usage-range: /);
  assert.match(refused(setAction(layout, 'main', 'a', { type: 'page', page: 'gone' })), /^action\.page-missing: /);
  assert.match(refused(setAction(layout, 'main', 'a', { type: 'launch', path: '', args: [], cwd: '' })), /^text\.empty: /);
  assert.equal(refused(setAction(layout, 'main', 'zz', { type: 'none' })), 'no tile zz');
});

test('a refused edit names the refused value by pointer, and empty when the rule names none', () => {
  const layout = small();
  const empty = setAction(layout, 'main', 'b', { type: 'command', program: '', args: [], cwd: '', shell: false });
  assert.deepEqual(empty.ok ? null : empty.path, '/pages/0/tiles/1/action/program');
  const missing = setAction(layout, 'main', 'a', { type: 'page', page: 'gone' });
  assert.deepEqual(missing.ok ? null : missing.path, '');
  assert.deepEqual(
    validateLayout({ ...layout, pages: 'none' }).map((error) => [error.key, error.path]),
    [['shape.type', '/pages']],
  );
});

test('labels and icons follow the schema', () => {
  const layout = small();
  assert.equal(grid(ok(setLabel(layout, 'main', 'a', 'Build')), 'main').tiles[0].label, 'Build');
  assert.match(refused(setLabel(layout, 'main', 'a', ' ')), /^label\.blank: /);
  assert.match(refused(setLabel(layout, 'main', 'a', 'seventeen letters')), /^label\.too-long: /);
  ok(setIcon(layout, 'main', 'a', 'lv:mute'));
  assert.match(refused(setIcon(layout, 'main', 'a', 'lv:nope')), /^icon\.format: /);
});

test('an Asset on a Tile makes the Layout schema 2 until no Tile uses one', () => {
  const id = 'ab'.repeat(32);
  const withAsset = ok(setAsset(setIconOk(small(), 'lv:mute'), 'main', 'a', id));
  assert.equal(withAsset.schema, 2);
  assert.deepEqual([grid(withAsset, 'main').tiles[0].icon, grid(withAsset, 'main').tiles[0].asset], [null, `sha256:${id}`]);
  assert.equal(parseLayout(canonical(withAsset)).ok, true);
  const back = ok(setIcon(withAsset, 'main', 'a', 'lv:next'));
  assert.equal(back.schema, 1);
  assert.equal(Object.hasOwn(grid(back, 'main').tiles[0], 'asset'), false);
  assert.match(refused(setAsset(small(), 'main', 'a', 'not-a-hash')), /^icon\.format: /);
});

test('picking an icon for a GIF Tile makes it a still Tile', () => {
  const gif = ok(setAsset(small(), 'main', 'a', 'cd'.repeat(32)));
  const tiles = grid(gif, 'main').tiles;
  const { streams: _streams, input: _input, ...still0 } = tiles[0];
  tiles[0] = { ...still0, render: 'gif', icon: null, asset: `sha256:${'cd'.repeat(32)}` };
  assert.equal(parseLayout(canonical(gif)).ok, true);
  const still = ok(setAsset(gif, 'main', 'a', 'ef'.repeat(32)));
  assert.equal(Object.hasOwn(grid(still, 'main').tiles[0], 'render'), false);
  assert.equal(grid(still, 'main').tiles[0].asset, `sha256:${'ef'.repeat(32)}`);
  const glyph = ok(setIcon(gif, 'main', 'a', 'lv:mute'));
  assert.deepEqual([grid(glyph, 'main').tiles[0].icon, Object.hasOwn(grid(glyph, 'main').tiles[0], 'render'), glyph.schema], ['lv:mute', false, 1]);
});

test('a graph Tile keeps the Layout schema 2 while other Tiles change icons, and going back to a label lets it drop', () => {
  const cpu = { integration: 'this_pc', stream: 'cpu_pct' };
  const graphed = ok(setGraph(small(), 'main', 'b', cpu));
  assert.equal(graphed.schema, 2);
  const iconed = ok(setIcon(graphed, 'main', 'a', 'lv:mute'));
  assert.equal(iconed.schema, 2);
  assert.deepEqual(grid(iconed, 'main').tiles[1].streams, { value: 'number' });
  assert.equal(parseLayout(canonical(iconed)).ok, true);
  const plain = ok(setGraph(iconed, 'main', 'b', null));
  assert.deepEqual([plain.schema, Object.hasOwn(grid(plain, 'main').tiles[1], 'streams')], [1, false]);
  assert.equal(parseLayout(canonical(plain)).ok, true);
});

function setIconOk(layout: Layout, icon: string): Layout {
  return ok(setIcon(layout, 'main', 'a', icon));
}

test('removeTile drops only that tile', () => {
  assert.deepEqual(
    grid(ok(removeTile(small(), 'main', 'a')), 'main').tiles.map((t) => t.id),
    ['b'],
  );
  assert.equal(refused(removeTile(small(), 'main', 'a2')), 'no tile a2');
});

test('addPage accepts up to eight and refuses a ninth', () => {
  let layout = small();
  for (let n = 3; n <= 8; n++) layout = ok(addPage(layout, newGridPage(`p${n}`, `P${n}`)));
  assert.equal(layout.pages.length, 8);
  assert.equal(refused(addPage(layout, newGridPage('p9', 'P9'))), 'a layout holds at most 8 pages');
  assert.match(refused(addPage(small(), newGridPage('main', 'MAIN'))), /^id\.duplicate: /);
});

test('a removed page clears the page actions that pointed to it', () => {
  const layout = ok(setAction(small(), 'main', 'a', { type: 'page', page: 'other' }));
  const removed = ok(removePage(layout, 'other'));
  assert.equal(removed.pages.length, 1);
  assert.deepEqual(grid(removed, 'main').tiles[0].action, { type: 'none' });
  assert.equal(refused(removePage(removed, 'main')), 'a layout keeps at least 1 page');
  assert.equal(refused(removePage(layout, 'gone')), 'no page gone');
});

test('dev.json edits keep its frozen ids', () => {
  const layout = dev();
  const page = layout.pages[0].id;
  const cell = freeCell(layout, page);
  assert.ok(cell, "dev.json's first page has a free cell");
  const id = freshId(layout, 'tile');
  const added = ok(addTile(layout, page, newTile(id, ...cell)));
  assert.equal(freshId(added, 'tile'), 'tile2');
  assert.match(refused(addTile(added, page, newTile('other', ...cell))), /^overlaps /);
  const dashboard = layout.pages.find((p) => p.kind === 'dashboard')!.id;
  assert.equal(refused(addTile(layout, dashboard, newTile('x', 0, 0))), `page ${dashboard} is a dashboard`);
  assert.ok(ok(removePage(layout, dashboard)).pages.every((p) => p.id !== dashboard));
});

test('full screen is written only when on, sits before the pages, and must be a boolean', () => {
  const on = setFullScreen(dev(), true);
  assert.ok(on.ok);
  assert.equal(on.layout.fullScreen, true);
  assert.deepEqual(Object.keys(on.layout), ['schema', 'name', 'orientation', 'fullScreen', 'pages']);
  assert.deepEqual(validateSource(canonical(on.layout)), []);
  const off = setFullScreen(on.layout, false);
  assert.ok(off.ok);
  assert.equal(Object.hasOwn(off.layout, 'fullScreen'), false);
  assert.equal(canonical(off.layout), canonical(dev()));
  assert.deepEqual(
    validateLayout({ ...dev(), fullScreen: 'yes' }).map((error) => error.key),
    ['shape.type'],
  );
  assert.deepEqual(validateLayout({ ...dev(), fullScreen: false }), []);
});

test('the starter layout passes the rules in both orientations', () => {
  for (const orientation of ['landscape', 'portrait'] as const) {
    assert.deepEqual(validateLayout(starterLayout(orientation)), []);
  }
});

test('a page and a layout can be renamed within their limits', () => {
  const named = setPageTitle(dev(), 'shortcuts', 'Keys');
  assert.ok(named.ok && named.layout.pages[0].title === 'Keys');
  const long = setPageTitle(dev(), 'shortcuts', 'x'.repeat(25));
  assert.ok(!long.ok && long.reason.startsWith('title.too-long'));
  const layout = setLayoutName(dev(), 'Desk');
  assert.ok(layout.ok && layout.layout.name === 'Desk');
});

test('a grid grows freely and shrinks only past empty cells', () => {
  const bigger = setGridSize(dev(), 'shortcuts', 4, 5);
  assert.ok(bigger.ok);
  const page = gridPage(bigger.layout.pages[0]);
  assert.deepEqual([page.rows, page.cols], [4, 5]);
  const smaller = setGridSize(dev(), 'shortcuts', 2, 3);
  assert.ok(!smaller.ok);
  assert.equal(smaller.reason, 'grid.too-small: quiet');
  assert.deepEqual(setGridSize(dev(), 'shortcuts', 9, 4), { ok: false, reason: 'grid.size', path: '' });
});
