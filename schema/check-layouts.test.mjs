import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { checkLayouts, pathKey, renderers, ruleKeys, schemaNodes, validateLayout, validateSource } from './check-layouts.mjs';

const directory = dirname(fileURLToPath(import.meta.url));
const read = (path) => JSON.parse(readFileSync(join(directory, path), 'utf8'));
const builtin = read('examples/builtin.json');
const dev = read('examples/dev.json');
const schema = read('layout.schema.json');
const copy = () => structuredClone(dev);
const keys = (layout) => validateLayout(layout).map(({ key }) => key);
const messages = (layout) => validateLayout(layout).map(({ message }) => message);
const sourceKeys = (text) => validateSource(Buffer.from(text)).map(({ key }) => key);
const invalidDirectory = join(directory, 'examples/invalid');
const invalidFiles = readdirSync(invalidDirectory).filter((name) => name.endsWith('.json'));
const splitExpectation = (bytes) => {
  const end = bytes.indexOf(0x0a);
  return { expected: bytes.subarray(0, end).toString('utf8').replace(/\r$/, '').replace(/^expected: /, ''), body: bytes.subarray(end + 1) };
};

for (const name of ['builtin', 'dev', 'full-screen', 'gif', 'graph', 'media']) {
  test(`${name} is valid without mutating the layout`, () => {
    const layout = read(`examples/${name}.json`);
    const before = structuredClone(layout);
    assert.deepEqual(validateLayout(layout), []);
    assert.deepEqual(layout, before);
    assert.deepEqual(validateSource(readFileSync(join(directory, `examples/${name}.json`))), []);
  });
}

for (const filename of invalidFiles) {
  test(`invalid example ${filename} fails with exactly its own rule key`, () => {
    const { expected, body } = splitExpectation(readFileSync(join(invalidDirectory, filename)));
    assert.ok(ruleKeys.has(expected), `${filename} names unknown key ${expected}`);
    assert.deepEqual(validateSource(body).map(({ key }) => key), [expected]);
  });
}

test('every rule key has an invalid example', () => {
  const covered = new Set(invalidFiles.map((filename) => splitExpectation(readFileSync(join(invalidDirectory, filename))).expected));
  assert.deepEqual([...ruleKeys].filter((key) => !covered.has(key)), []);
});

test('schema/README.md lists exactly the rule keys', () => {
  const readme = readFileSync(join(directory, 'README.md'), 'utf8');
  const section = readme.slice(readme.indexOf('## Rule keys'));
  const listed = [...section.matchAll(/^\| `([a-z][a-z0-9.-]*)` \|/gm)].map(([, key]) => key);
  assert.equal(new Set(listed).size, listed.length);
  assert.deepEqual(new Set(listed), ruleKeys);
});

test('every constraint in the schema carries a rule key', () => {
  const enforced = ['const', 'enum', 'pattern', 'minimum', 'maximum', 'minLength', 'maxLength', 'minItems', 'maxItems', 'uniqueItems', 'oneOf'];
  for (const [node, property] of schemaNodes(schema)) {
    for (const keyword of enforced.filter((name) => name in node)) {
      if (keyword === 'const' && (property === 'kind' || property === 'type')) continue;
      assert.ok(node['x-rules']?.[keyword], `${keyword} on ${property ?? JSON.stringify(node).slice(0, 60)} has no rule key`);
    }
    for (const keyword of Object.keys(node['x-rules'] ?? {})) assert.ok(keyword in node, `x-rules names absent keyword ${keyword}`);
  }
});

test('builtin preserves all milestone 1 pages, positions, actions, and icons', () => {
  assert.deepEqual(builtin.pages.map(({ id, title, kind, rows, cols }) => ({ id, title, kind, rows, cols })), [
    { id: 'fkeys', title: 'F-Keys', kind: 'grid', rows: 4, cols: 4 },
    { id: 'media', title: 'Media', kind: 'grid', rows: 2, cols: 3 },
  ]);
  const tiles = builtin.pages.flatMap((page) => page.tiles);
  assert.equal(tiles.length, 22);
  for (let index = 0; index < 12; index++) {
    const tile = tiles[index];
    assert.equal(tile.id, `f${index + 1}`);
    assert.equal(tile.label, `F${index + 1}`);
    assert.deepEqual([tile.row, tile.col, tile.rowSpan, tile.colSpan], [Math.floor(index / 4), index % 4, 1, 1]);
    assert.equal(tile.icon, null);
    assert.deepEqual(tile.action, { type: 'key', usage: 58 + index, mods: [] });
  }
  for (const [index, mod] of ['shift', 'ctrl', 'alt', 'win'].entries()) {
    assert.equal(tiles[12 + index].id, mod);
    assert.deepEqual(tiles[12 + index].action, { type: 'mod', mod });
  }
  assert.deepEqual(builtin.pages[1].tiles.map(({ id, label, icon, action }) => [id, label, icon, action.usage]), [
    ['prev', 'Prev', 'lv:prev', 182],
    ['playpause', 'Play/Pause', 'lv:play_pause', 205],
    ['next', 'Next', 'lv:next', 181],
    ['mute', 'Mute', 'lv:mute', 226],
    ['voldown', 'Vol -', 'lv:volume_mid', 234],
    ['volup', 'Vol +', 'lv:volume_max', 233],
  ]);
});

test('dev exercises every action, both spanning directions, and all requested widgets', () => {
  assert.deepEqual(dev.pages.map((page) => page.kind), ['grid', 'grid', 'grid', 'dashboard']);
  const tiles = dev.pages.flatMap((page) => page.tiles ?? []);
  const actions = tiles.map((tile) => tile.action);
  assert.deepEqual(new Set(actions.map((action) => action.type)), new Set(schema.$defs.action.oneOf.map((action) => action.properties.type.const)));
  assert.equal(tiles.filter((tile) => tile.rowSpan > 1 || tile.colSpan > 1).length, 2);
  assert.ok(tiles.some((tile) => tile.rowSpan > 1));
  assert.ok(tiles.some((tile) => tile.colSpan > 1));
  assert.ok(actions.some((action) => action.type === 'key' && action.mods.length > 0));
  assert.ok(actions.some((action) => action.type === 'page' && action.page === dev.pages[3].id));
  assert.equal(actions.filter((action) => action.type === 'widget' && action.source === 'cliproxyapi').length, 4);
  assert.deepEqual(actions.find((action) => action.type === 'command'), { type: 'command', program: 'cmd', args: ['/c', 'echo', 'TouchDeck'], cwd: '', shell: false });
});

test('dev names no file icon and uses aggregate and provider-specific widget accounts', () => {
  const tiles = dev.pages.flatMap((page) => page.tiles ?? []);
  assert.ok(tiles.every((tile) => tile.icon === null || tile.icon.startsWith('lv:')));
  const widgets = tiles.map((tile) => tile.action).filter((action) => action.type === 'widget');
  assert.equal(widgets.length, 4);
  assert.equal(widgets.filter((action) => action.account === '').length, 3);
  assert.equal(widgets.find((action) => action.account !== '').account, 'claude:alice@example.com');
});

test('dev preserves the ids and roles used by later milestone lanes', () => {
  const tiles = new Map(dev.pages.flatMap((page) => (page.tiles ?? []).map((tile) => [tile.id, { page: page.id, action: tile.action }])));
  for (const id of ['f5', 'shift', 'chord', 'launch', 'command', 'open', 'five_hour', 'seven_day', 'claude_5h', 'proxy']) {
    assert.ok(tiles.has(id), `missing fixture tile ${id}`);
  }
  assert.deepEqual(tiles.get('f5').action, { type: 'key', usage: 62, mods: [] });
  assert.deepEqual(tiles.get('shift').action, { type: 'mod', mod: 'shift' });
  assert.equal(tiles.get('f5').page, tiles.get('shift').page);
  assert.equal(tiles.get('chord').action.type, 'key');
  assert.deepEqual(new Set(tiles.get('chord').action.mods), new Set(['ctrl', 'shift']));
  assert.equal(tiles.get('launch').action.type, 'launch');
  assert.equal(tiles.get('open').action.type, 'open');
  assert.deepEqual(tiles.get('command').action, { type: 'command', program: 'cmd', args: ['/c', 'echo', 'TouchDeck'], cwd: '', shell: false });
  for (const [id, source, field] of [
    ['five_hour', 'cliproxyapi', 'five_hour_pct'],
    ['seven_day', 'cliproxyapi', 'seven_day_pct'],
    ['claude_5h', 'cliproxyapi', 'five_hour_pct'],
    ['proxy', 'cliproxyapi', 'status'],
  ]) {
    const action = tiles.get(id).action;
    assert.deepEqual([action.type, action.source, action.field], ['widget', source, field]);
  }
  const accounts = dev.pages.find((page) => page.id === 'accounts');
  assert.deepEqual(accounts, { id: 'accounts', title: 'Accounts', kind: 'dashboard', source: 'cliproxyapi' });
  assert.ok([...tiles.values()].some(({ action }) => action.type === 'page' && action.page === 'accounts'));
});

test('overlap names both tiles once even when multiple cells overlap', () => {
  const layout = copy();
  const [first, second] = layout.pages[0].tiles;
  Object.assign(second, { row: first.row, col: first.col, colSpan: first.colSpan });
  assert.deepEqual(validateLayout(layout), [{ key: 'tile.overlap', message: 'page "shortcuts" tiles "chord" and "shift" overlap' }]);
});

test('overlap reports every pair when three tiles share a cell', () => {
  const layout = copy();
  const [first, second, third] = layout.pages[0].tiles;
  Object.assign(second, { row: first.row, col: first.col });
  Object.assign(third, { row: first.row, col: first.col });
  assert.deepEqual(messages(layout), [
    'page "shortcuts" tiles "chord" and "shift" overlap',
    'page "shortcuts" tiles "chord" and "quiet" overlap',
    'page "shortcuts" tiles "shift" and "quiet" overlap',
  ]);
});

test('duplicate tile ids across pages report both locations', () => {
  const layout = copy();
  layout.pages[1].tiles[0].id = 'chord';
  assert.deepEqual(validateLayout(layout), [{ key: 'id.duplicate', message: 'duplicate id "chord" at page 1 "desktop" tile 0 "chord"; first used at page 0 "shortcuts" tile 0 "chord"' }]);
});

test('duplicate tile ids on one page name both tile indexes', () => {
  const layout = copy();
  layout.pages[0].tiles[1].id = 'chord';
  assert.deepEqual(messages(layout), ['duplicate id "chord" at page 0 "shortcuts" tile 1 "chord"; first used at page 0 "shortcuts" tile 0 "chord"']);
});

test('page ids share the tile id namespace', () => {
  const layout = copy();
  layout.pages[0].tiles[1].id = 'accounts';
  assert.deepEqual(messages(layout), ['duplicate id "accounts" at page 3 "accounts"; first used at page 0 "shortcuts" tile 1 "accounts"']);
});

test('duplicate page ids name both page indexes', () => {
  const layout = copy();
  layout.pages[1].id = layout.pages[0].id;
  assert.deepEqual(messages(layout), ['duplicate id "shortcuts" at page 1 "shortcuts"; first used at page 0 "shortcuts"']);
});

test('missing page targets report page and tile', () => {
  const layout = copy();
  layout.pages[0].tiles[3].action.page = 'missing';
  assert.deepEqual(validateLayout(layout), [{ key: 'action.page-missing', message: 'page "shortcuts" tile "accounts_jump" references missing page "missing"' }]);
});

test('64 tiles fit and a 65th tile fails the tile count', () => {
  const layout = copy();
  layout.pages = [{ id: 'full', title: 'Full', kind: 'grid', rows: 8, cols: 8, tiles: Array.from({ length: 64 }, (_, index) => ({ id: `t_${index}`, label: 'Tile', row: Math.floor(index / 8), col: index % 8, rowSpan: 1, colSpan: 1, icon: null, action: { type: 'none' } })) }];
  assert.deepEqual(validateLayout(layout), []);
  layout.pages[0].tiles.push({ ...layout.pages[0].tiles[0], id: 't_64' });
  assert.deepEqual(keys(layout), ['page.tile-count']);
});

for (const [name, mutate, key] of [
  ['unsupported version', (x) => { x.schema = 3; }, 'layout.schema-version'],
  ['string version', (x) => { x.schema = '1'; }, 'layout.schema-version'],
  ['unknown orientation', (x) => { x.orientation = 'upside_down'; }, 'layout.orientation'],
  ['no pages', (x) => { x.pages = []; }, 'layout.page-count'],
  ['nine pages', (x) => { x.pages = Array.from({ length: 9 }, (_, n) => ({ id: `p_${n}`, title: 'Page', kind: 'grid', rows: 1, cols: 1, tiles: [] })); }, 'layout.page-count'],
  ['invalid id', (x) => { x.pages[0].id = 'Upper-Case'; }, 'id.format'],
  ['16-character id', (x) => { x.pages[0].tiles[0].id = 'a'.repeat(16); }, 'id.format'],
  ['empty grid title', (x) => { x.pages[0].title = ''; }, 'title.blank'],
  ['empty dashboard title', (x) => { x.pages[3].title = ''; }, 'title.blank'],
  ['empty tile label', (x) => { x.pages[0].tiles[0].label = ''; }, 'label.blank'],
  ['17-character label', (x) => { x.pages[0].tiles[0].label = 'a'.repeat(17); }, 'label.too-long'],
  ['17-code-point label', (x) => { x.pages[0].tiles[0].label = '😀'.repeat(17); }, 'label.too-long'],
  ['25-character title', (x) => { x.pages[0].title = 'a'.repeat(25); }, 'title.too-long'],
  ['25-code-point title', (x) => { x.pages[0].title = '😀'.repeat(25); }, 'title.too-long'],
  ['zero rows', (x) => { x.pages[0].rows = 0; }, 'grid.size'],
  ['nine cols', (x) => { x.pages[0].cols = 9; }, 'grid.size'],
  ['fractional coordinate', (x) => { x.pages[0].tiles[0].row = 0.5; }, 'shape.type'],
  ['negative coordinate', (x) => { x.pages[0].tiles[0].col = -1; }, 'tile.position'],
  ['coordinate 8', (x) => { x.pages[0].tiles[0].row = 8; }, 'tile.position'],
  ['zero span', (x) => { x.pages[0].tiles[0].rowSpan = 0; }, 'tile.span'],
  ['span 9', (x) => { x.pages[0].tiles[0].colSpan = 9; }, 'tile.span'],
  ['row outside page', (x) => { x.pages[0].tiles[0].row = 3; }, 'tile.bounds'],
  ['column span outside page', (x) => { x.pages[0].tiles[0].colSpan = 5; }, 'tile.bounds'],
  ['key usage zero', (x) => { x.pages[0].tiles[0].action.usage = 0; }, 'key.usage-range'],
  ['key usage 256', (x) => { x.pages[0].tiles[0].action.usage = 256; }, 'key.usage-range'],
  ['consumer usage zero', (x) => { x.pages[0].tiles[2].action.usage = 0; }, 'consumer.usage-range'],
  ['consumer usage 65536', (x) => { x.pages[0].tiles[2].action.usage = 65536; }, 'consumer.usage-range'],
  ['repeated modifier', (x) => { x.pages[0].tiles[0].action.mods = ['ctrl', 'ctrl']; }, 'key.mods-duplicate'],
  ['unknown modifier', (x) => { x.pages[0].tiles[1].action.mod = 'hyper'; }, 'modifier.unknown'],
  ['unknown modifier in a chord', (x) => { x.pages[0].tiles[0].action.mods = ['hyper']; }, 'modifier.unknown'],
  ['unknown action', (x) => { x.pages[0].tiles[0].action.type = 'macro'; }, 'action.type'],
  ['missing action field', (x) => { delete x.pages[0].tiles[0].action.mods; }, 'shape.missing-field'],
  ['unknown icon', (x) => { x.pages[0].tiles[0].icon = 'lv:unknown'; }, 'icon.format'],
  ['short bitmap hash', (x) => { x.pages[0].tiles[0].icon = 'file:123'; }, 'icon.format'],
  ['numeric icon', (x) => { x.pages[0].tiles[0].icon = 5; }, 'shape.type'],
  ['dashboard with tiles', (x) => { x.pages[3].tiles = []; }, 'shape.unknown-field'],
  ['dashboard with grid size', (x) => { x.pages[3].rows = 1; }, 'shape.unknown-field'],
  ['dashboard with invalid tiles', (x) => { x.pages[3].tiles = [{ id: 'x' }]; }, 'shape.unknown-field'],
  ['grid with empty source', (x) => { x.pages[0].source = ''; }, 'shape.unknown-field'],
  ['open action with empty launch path', (x) => { x.pages[1].tiles[1].action.path = ''; }, 'shape.unknown-field'],
  ['missing grid kind', (x) => { delete x.pages[0].kind; }, 'page.kind'],
  ['missing dashboard kind', (x) => { delete x.pages[3].kind; }, 'page.kind'],
  ['unknown kind with invalid tiles', (x) => { x.pages[0].kind = 'carousel'; x.pages[0].tiles[0].label = ''; }, 'page.kind'],
  ['missing action type', (x) => { delete x.pages[0].tiles[0].action.type; }, 'action.type'],
  ['unknown action type with other fields invalid', (x) => { x.pages[0].tiles[0].action.type = 'macro'; x.pages[0].tiles[0].action.usage = 0; }, 'action.type'],
  ['dashboard without source', (x) => { delete x.pages[3].source; }, 'shape.missing-field'],
  ['empty dashboard source', (x) => { x.pages[3].source = ''; }, 'text.empty'],
  ['nonstring command argument', (x) => { x.pages[1].tiles[2].action.args = [1]; }, 'shape.type'],
  ['nonboolean shell', (x) => { x.pages[1].tiles[2].action.shell = 'cmd'; }, 'shape.type'],
  ['empty program', (x) => { x.pages[1].tiles[2].action.program = ''; }, 'text.empty'],
  ['null page', (x) => { x.pages[0] = null; }, 'shape.type'],
  ['null action', (x) => { x.pages[0].tiles[0].action = null; }, 'shape.type'],
]) {
  test(`rejects ${name} with ${key}`, () => {
    const layout = copy();
    mutate(layout);
    assert.deepEqual(keys(layout), [key], messages(layout).join('; '));
  });
}

for (const [name, select] of [
  ['layout', (x) => x],
  ['grid page', (x) => x.pages[0]],
  ['dashboard page', (x) => x.pages[3]],
  ['tile', (x) => x.pages[0].tiles[0]],
  ...schema.$defs.action.oneOf.map(({ properties }) => [properties.type.const, (x) => x.pages.flatMap((page) => page.tiles ?? []).find((tile) => tile.action.type === properties.type.const).action]),
]) {
  test(`rejects additional properties on ${name}`, () => {
    const layout = copy();
    select(layout).extra = true;
    assert.deepEqual(validateLayout(layout).map(({ key, message }) => [key, message.endsWith('"extra"')]), [['shape.unknown-field', true]]);
  });
}

test('accepts all inclusive limits, unicode character limits, portrait, and an empty grid', () => {
  const layout = copy();
  layout.orientation = 'portrait';
  layout.name = '😀'.repeat(24);
  layout.pages[0].tiles[0].id = 'a'.repeat(15);
  layout.pages[0].tiles[0].label = '😀'.repeat(16);
  layout.pages[0].title = '😀'.repeat(24);
  layout.pages[0].tiles[0].action.usage = 255;
  layout.pages[0].tiles[0].action.mods = ['ctrl', 'shift', 'alt', 'win'];
  layout.pages[0].tiles[2].action.usage = 65535;
  layout.pages[0].tiles[0].icon = 'file:abcdef0123456789';
  layout.pages.push(...Array.from({ length: 4 }, (_, n) => ({ id: `empty_${n}`, title: 'Empty', kind: 'grid', rows: 1, cols: 1, tiles: [] })));
  assert.deepEqual(validateLayout(layout), []);
});

test('fullScreen is optional and must be a boolean', () => {
  const layout = copy();
  assert.equal('fullScreen' in layout, false);
  for (const value of [true, false]) {
    layout.fullScreen = value;
    assert.deepEqual(validateLayout(layout), []);
  }
  for (const value of [1, 0, 'true', null, [], {}]) {
    layout.fullScreen = value;
    assert.deepEqual(keys(layout), ['shape.type'], JSON.stringify(value));
  }
  assert.equal(read('examples/full-screen.json').fullScreen, true);
});

for (const layout of [null, [], 'layout', {}, { ...dev, pages: [null] }, { ...dev, pages: [{ ...dev.pages[0], tiles: [null] }] }]) {
  test(`rejects malformed shape ${JSON.stringify(layout).slice(0, 70)} without throwing`, () => {
    assert.ok(validateLayout(layout).length > 0);
  });
}

test('rejects invalid UTF-8 and accepts multibyte UTF-8', () => {
  const marked = Buffer.from(JSON.stringify(dev).replace('"F5"', '"F5@"'));
  const at = marked.indexOf('F5@') + 2;
  const withBytes = (bytes) => Buffer.concat([marked.subarray(0, at), Buffer.from(bytes), marked.subarray(at + 1)]);
  assert.deepEqual(validateSource(withBytes('é')), []);
  for (const bytes of [[0xe9], [0xc0, 0xaf], [0xed, 0xa0, 0x80], [0xf4, 0x90, 0x80, 0x80], [0xff]]) {
    assert.deepEqual(validateSource(withBytes(bytes)).map(({ key }) => key), ['file.utf8'], `bytes ${bytes}`);
  }
});

test('rejects a byte order mark and syntax errors as invalid JSON', () => {
  const text = JSON.stringify(dev);
  assert.deepEqual(sourceKeys(text), []);
  assert.deepEqual(validateSource(Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), Buffer.from(text)])).map(({ key }) => key), ['file.json']);
  assert.deepEqual(sourceKeys(text.slice(0, -1)), ['file.json']);
  assert.deepEqual(sourceKeys(text.replace('"schema":1', '"schema":1,')), ['file.json']);
});

test('rejects lone surrogates in values and member names, and accepts surrogate pairs', () => {
  const layout = copy();
  layout.pages[0].tiles[0].label = '😀';
  assert.deepEqual(validateLayout(layout), []);
  for (const lone of ['\ud800', '\udc00', 'a\ud83d', '\ude00b']) {
    const bad = copy();
    bad.pages[0].tiles[0].label = lone;
    assert.deepEqual(keys(bad), ['string.malformed-unicode'], JSON.stringify(lone));
  }
  const key = copy();
  key.pages[0]['\ud800'] = true;
  assert.ok(keys(key).includes('string.malformed-unicode'));
  const text = JSON.stringify(dev).replace('"F5"', '"\\ud800"');
  assert.deepEqual(sourceKeys(text), ['string.malformed-unicode']);
});

test('rejects duplicate member names at every depth and accepts repeated names in different objects', () => {
  const text = JSON.stringify(dev);
  assert.deepEqual(sourceKeys(text), []);
  assert.deepEqual(sourceKeys(text.replace('"schema":1', '"schema":1,"schema":1')), ['json.duplicate-member']);
  assert.deepEqual(sourceKeys(text.replace('"usage":62', '"usage":300,"usage":62')), ['json.duplicate-member']);
  assert.deepEqual(sourceKeys(text.replace('"type":"none"', '"type":"key","type":"none"')), ['json.duplicate-member']);
  assert.deepEqual(sourceKeys(text.replace('"id":"f5"', '"id":"f5","\\u0069d":"f5"')), ['json.duplicate-member']);
  assert.deepEqual(validateSource(Buffer.from(text.replace('"usage":62', '"usage":300,"usage":62'))).map(({ message }) => message), ['/pages/0/tiles/5/action repeats the member name "usage"']);
  const labelled = JSON.parse(text);
  labelled.pages[0].tiles[0].label = '{"id":1,"id":2}';
  assert.deepEqual(sourceKeys(JSON.stringify(labelled)), []);
  labelled.pages[0].tiles[0].label = '"id":"id"';
  assert.deepEqual(sourceKeys(JSON.stringify(labelled)), []);
});

test('rejects integers written with a fraction or an exponent', () => {
  const text = JSON.stringify(dev);
  assert.deepEqual(sourceKeys(text), []);
  for (const [from, to, pointer] of [
    ['"schema":1', '"schema":1.0', '/schema'],
    ['"rows":3', '"rows":3.0', '/pages/0/rows'],
    ['"usage":62', '"usage":6.2e1', '/pages/0/tiles/5/action/usage'],
    ['"usage":62', '"usage":62E0', '/pages/0/tiles/5/action/usage'],
    ['"rowSpan":2', '"rowSpan":2.00', '/pages/1/tiles/0/rowSpan'],
  ]) {
    assert.ok(text.includes(from), from);
    assert.deepEqual(validateSource(Buffer.from(text.replace(from, to))).map(({ key, message }) => [key, message.split(' ')[0]]), [['json.integer-form', pointer]], to);
  }
  const labelled = JSON.parse(text);
  labelled.pages[0].tiles[0].label = '1.5e3';
  assert.deepEqual(sourceKeys(JSON.stringify(labelled)), []);
});

test('rejects negative zero as an integer and accepts zero and other negatives as integers', () => {
  const text = JSON.stringify(dev);
  assert.ok(text.includes('"row":0,"col":0'));
  assert.deepEqual(validateSource(Buffer.from(text.replace('"row":0,"col":0', '"row":-0,"col":0'))).map(({ key, message }) => [key, message.split(' ')[0]]), [['json.integer-form', '/pages/0/tiles/0/row']]);
  assert.deepEqual(validateSource(Buffer.from(text.replace('"row":0,"col":0', '"row":0,"col":-0'))).map(({ key, message }) => [key, message.split(' ')[0]]), [['json.integer-form', '/pages/0/tiles/0/col']]);
  assert.deepEqual(sourceKeys(text.replace('"row":0,"col":0', '"row":-1,"col":0')), ['tile.position']);
  assert.deepEqual(sourceKeys(text.replace('"row":0,"col":0', '"row":-10,"col":0')), ['tile.position']);
});

const nestedName = (levels) => JSON.stringify(dev).replace(/"name":"[^"]*"/, `"name":${'['.repeat(levels)}${']'.repeat(levels)}`);

test('rejects JSON nested deeper than 16 levels as file.json and never throws', () => {
  const sixteen = validateSource(Buffer.from(nestedName(15)));
  assert.ok(sixteen.length > 0);
  assert.ok(!sixteen.some(({ key }) => key === 'file.json'), JSON.stringify(sixteen));
  for (const levels of [16, 100, 1999, 100000]) {
    const errors = validateSource(Buffer.from(nestedName(levels)));
    assert.deepEqual(errors.map(({ key }) => key), ['file.json'], `${levels + 1} levels`);
    assert.match(errors[0].message, /16 levels/);
  }
  assert.deepEqual(validateSource(Buffer.from('['.repeat(17) + ']'.repeat(17))).map(({ key }) => key), ['file.json']);
  assert.ok(!validateSource(Buffer.from('['.repeat(16) + ']'.repeat(16))).some(({ message }) => /16 levels/.test(message)));
  assert.deepEqual(sourceKeys(JSON.stringify({ ...dev, name: '[[[[[[[[[[[[[[[[[[[[' })), []);
});

test('validateLayout never throws on a deeply nested parsed value', () => {
  let value = 'x';
  for (let level = 0; level < 100000; level++) value = [value];
  const layout = copy();
  layout.name = value;
  assert.ok(keys(layout).includes('shape.type'));
  layout.name = 'Deep';
  layout.pages[0].tiles[0].action = { type: 'none', extra: value };
  assert.deepEqual(keys(layout), ['shape.unknown-field']);
});

const textFields = [
  ['name', 'name', (x, v) => { x.name = v; }],
  ['title', 'title', (x, v) => { x.pages[0].title = v; }],
  ['label', 'label', (x, v) => { x.pages[0].tiles[0].label = v; }],
  ['dashboard source', null, (x, v) => { x.pages[3].source = v; }],
  ['launch path', null, (x, v) => { x.pages[1].tiles[0].action.path = v; }],
  ['launch argument', null, (x, v) => { x.pages[1].tiles[0].action.args = [v]; }],
  ['launch cwd', null, (x, v) => { x.pages[1].tiles[0].action.cwd = v; }],
  ['open target', null, (x, v) => { x.pages[1].tiles[1].action.target = v; }],
  ['command program', null, (x, v) => { x.pages[1].tiles[2].action.program = v; }],
  ['command argument', null, (x, v) => { x.pages[1].tiles[2].action.args = ['/c', v]; }],
  ['command cwd', null, (x, v) => { x.pages[1].tiles[2].action.cwd = v; }],
  ['widget source', null, (x, v) => { x.pages[2].tiles[0].action.source = v; }],
  ['widget field', null, (x, v) => { x.pages[2].tiles[0].action.field = v; }],
  ['widget account', null, (x, v) => { x.pages[2].tiles[0].action.account = v; }],
];

for (const [name, , set] of textFields) {
  test(`${name} rejects control characters and accepts space and C1 characters`, () => {
    for (const control of ['\u0000', '\u0001', '\n', '\u001f', '\u007f']) {
      const layout = copy();
      set(layout, `a${control}b`);
      assert.deepEqual(keys(layout), ['string.control-character'], JSON.stringify(control));
    }
    const layout = copy();
    set(layout, 'a b\u0080\u0085');
    assert.deepEqual(validateLayout(layout), []);
  });
}

for (const [name, prefix, set] of textFields.filter(([, prefix]) => prefix)) {
  test(`${name} needs a non-whitespace character`, () => {
    for (const blank of ['', ' ', ' ', '　', '   ']) {
      const layout = copy();
      set(layout, blank);
      assert.deepEqual(keys(layout), [`${prefix}.blank`], JSON.stringify(blank));
    }
    const tab = copy();
    set(tab, '\t');
    assert.deepEqual(keys(tab), ['string.control-character', `${prefix}.blank`]);
    for (const visible of ['a', ' a ', '​', '.']) {
      const layout = copy();
      set(layout, visible);
      assert.deepEqual(validateLayout(layout), [], JSON.stringify(visible));
    }
  });
}

test('name holds at most 24 code points', () => {
  const layout = copy();
  for (const name of ['a'.repeat(24), '😀'.repeat(24)]) {
    layout.name = name;
    assert.deepEqual(validateLayout(layout), []);
  }
  for (const name of ['a'.repeat(25), '😀'.repeat(25), 'n'.repeat(5000)]) {
    layout.name = name;
    assert.deepEqual(keys(layout), ['name.too-long']);
  }
});

test('file icon ids are 16 lowercase hexadecimal digits', () => {
  const layout = copy();
  const icon = (value) => { layout.pages[0].tiles[0].icon = value; return keys(layout); };
  assert.deepEqual(icon('file:0123456789abcdef'), []);
  assert.deepEqual(icon('file:ffffffffffffffff'), []);
  for (const bad of ['file:0123456789ABCDEF', 'file:0123456789AbCdEf', 'file:0123456789abcde', 'file:0123456789abcdef0', 'file:0123456789abcdeg', 'FILE:0123456789abcdef', ' file:0123456789abcdef']) {
    assert.deepEqual(icon(bad), ['icon.format'], bad);
  }
});

test('size limit applies to the canonical compact form, not to the raw bytes', () => {
  const layout = copy();
  const action = layout.pages[1].tiles[1].action;
  const escapedSource = () => Buffer.from(JSON.stringify(layout).replaceAll('F', `${String.fromCharCode(92)}u0046`));
  action.target = '';
  action.target = 'F'.repeat(32768 - Buffer.byteLength(JSON.stringify(layout), 'utf8'));
  assert.equal(Buffer.byteLength(JSON.stringify(layout), 'utf8'), 32768);
  assert.ok(escapedSource().length > 5 * 32768);
  assert.deepEqual(validateSource(escapedSource()), []);
  assert.deepEqual(validateSource(Buffer.from(JSON.stringify(layout, null, 4))), []);
  action.target += 'F';
  assert.deepEqual(validateSource(escapedSource()).map(({ key }) => key), ['layout.too-large']);
});

test('widget account is required and may be empty', () => {
  const widget = (layout) => layout.pages[2].tiles[0].action;
  const layout = copy();
  widget(layout).account = '';
  assert.deepEqual(validateLayout(layout), []);
  widget(layout).account = 'someone@example.com';
  assert.deepEqual(validateLayout(layout), []);
  delete widget(layout).account;
  assert.deepEqual(keys(layout), ['shape.missing-field']);
  widget(layout).account = 3;
  assert.deepEqual(keys(layout), ['shape.type']);
});

test('default checker reports each fixture and the exact acceptance summary', () => {
  const lines = [];
  assert.equal(checkLayouts([], (line) => lines.push(line)), 0);
  assert.equal(lines.filter((line) => line.startsWith('ok ')).length, 7 + invalidFiles.length);
  assert.equal(lines.at(-1), `7 valid, ${invalidFiles.length} invalid as expected, 0 problems`);
});

test('compact UTF-8 JSON accepts exactly 32768 bytes and rejects 32769 bytes', () => {
  const layout = copy();
  const action = layout.pages[1].tiles[1].action;
  action.target = '😀 "\\';
  const initialBytes = Buffer.byteLength(JSON.stringify(layout), 'utf8');
  action.target += 'x'.repeat(32768 - initialBytes);
  assert.equal(Buffer.byteLength(JSON.stringify(layout), 'utf8'), 32768);
  assert.ok(JSON.stringify(layout).length < 32768);
  assert.deepEqual(validateLayout(layout), []);
  action.target += 'x';
  assert.equal(Buffer.byteLength(JSON.stringify(layout), 'utf8'), 32769);
  assert.deepEqual(validateLayout(layout), [{ key: 'layout.too-large', message: 'layout compact UTF-8 JSON is 32769 bytes; maximum is 32768 bytes' }]);
});

test('CLI ignores formatting whitespace and counts whitespace inside strings', () => {
  const temporary = mkdtempSync(join(tmpdir(), 'touchdeck-size-'));
  const path = join(temporary, 'boundary.json');
  const layout = copy();
  const action = layout.pages[1].tiles[1].action;
  action.target = ' ';
  action.target += ' '.repeat(32768 - Buffer.byteLength(JSON.stringify(layout), 'utf8'));
  const run = () => spawnSync(process.execPath, [resolve(directory, 'check-layouts.mjs'), path], { encoding: 'utf8' });
  try {
    const source = JSON.stringify(layout, null, 2) + '\n';
    assert.ok(Buffer.byteLength(source, 'utf8') > 32768);
    writeFileSync(path, source);
    let result = run();
    assert.equal(result.status, 0, result.stdout + result.stderr);
    assert.match(result.stdout, new RegExp(`8 valid, ${invalidFiles.length} invalid as expected, 0 problems`));
    action.target += ' ';
    writeFileSync(path, JSON.stringify(layout));
    result = run();
    assert.equal(result.status, 1);
    assert.match(result.stdout, /layout.too-large: layout compact UTF-8 JSON is 32769 bytes; maximum is 32768 bytes/);
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
});

test('CLI checks extra paths, rejects JSON and file errors, and runs from another directory', () => {
  const temporary = mkdtempSync(join(tmpdir(), 'touchdeck-schema-'));
  const run = (...paths) => spawnSync(process.execPath, [resolve(directory, 'check-layouts.mjs'), ...paths], { cwd: temporary, encoding: 'utf8' });
  try {
    const good = join(temporary, 'good.json');
    writeFileSync(good, JSON.stringify(dev));
    let result = run(good);
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, new RegExp(`8 valid, ${invalidFiles.length} invalid as expected, 0 problems`));
    assert.match(result.stdout, /wall time: \d+\.\d+ ms/);
    const bad = join(temporary, 'bad.json');
    writeFileSync(bad, splitExpectation(readFileSync(join(invalidDirectory, 'overlap.json'))).body);
    result = run(bad);
    assert.equal(result.status, 1);
    assert.match(result.stdout, /tile.overlap: .*"alpha" and "beta" overlap/);
    writeFileSync(bad, readFileSync(join(invalidDirectory, 'overlap.json')));
    result = run(bad);
    assert.equal(result.status, 1);
    assert.match(result.stdout, /file\.json/);
    writeFileSync(bad, '{');
    result = run(bad);
    assert.equal(result.status, 1);
    assert.match(result.stdout, /file\.json/);
    assert.match(result.stdout, /1 problems/);
    writeFileSync(bad, Buffer.from([0x7b, 0x22, 0xe9, 0x22, 0x3a, 0x31, 0x7d]));
    result = run(bad);
    assert.equal(result.status, 1);
    assert.match(result.stdout, /file\.utf8/);
    result = run(join(temporary, 'missing.json'));
    assert.equal(result.status, 1);
    assert.match(result.stdout, /ENOENT/);
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
});

test('path keys fold case only on Windows', () => {
  assert.equal(pathKey('C:\Schema\Examples\Invalid\a.json', 'win32'), 'c:\schema\examples\invalid\a.json');
  assert.equal(pathKey('/schema/Examples/Invalid/a.json', 'linux'), '/schema/Examples/Invalid/a.json');
});

test('CLI dedupes paths and finds invalid/ whatever the drive-letter and folder case', { skip: process.platform !== 'win32' }, () => {
  const flipDrive = (path) => (path[0] === path[0].toLowerCase() ? path[0].toUpperCase() : path[0].toLowerCase()) + path.slice(1);
  const result = spawnSync(
    process.execPath,
    [resolve(directory, 'check-layouts.mjs'), flipDrive(join(directory, 'EXAMPLES', 'Invalid', 'overlap.json')), flipDrive(join(directory, 'Examples', 'DEV.json'))],
    { encoding: 'utf8' },
  );
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.match(result.stdout, new RegExp(`7 valid, ${invalidFiles.length} invalid as expected, 0 problems`));
});

test('schema 2 accepts Asset references and forbids mixing icons', () => {
  const layout = copy();
  layout.schema = 2;
  const tile = layout.pages[0].tiles[0];
  tile.icon = null;
  tile.asset = `sha256:${'a'.repeat(64)}`;
  assert.deepEqual(keys(layout), []);
  tile.icon = 'file:0123456789abcdef';
  assert.ok(keys(layout).includes('icon.format'));
  tile.icon = null;
  tile.asset = `sha256:${'A'.repeat(64)}`;
  assert.ok(keys(layout).includes('icon.format'));
  tile.asset = null;
  assert.deepEqual(keys(layout), []);
  layout.schema = 1;
  assert.ok(keys(layout).includes('shape.unknown-field'));
});

test('bar and now-playing Tiles bind the streams the shared renderer table lists', () => {
  const media = read('examples/media.json');
  const [volume, level, playing] = media.pages[0].tiles;
  assert.deepEqual(Object.keys(renderers), ['graph', 'bar', 'now-playing']);
  assert.deepEqual(schemaNodes(read('layout.schema.json')).next().value[0].$defs.tile.properties.render.enum, ['gif', ...Object.keys(renderers)]);
  assert.deepEqual(keys(media), []);
  const required = (render) => Object.entries(renderers[render].streams).filter(([, stream]) => stream.required).map(([id]) => id);
  assert.deepEqual(required('bar'), ['value']);
  assert.deepEqual(required('now-playing'), ['title', 'artist', 'cover', 'state']);
  assert.ok(!Object.hasOwn(level.streams, 'muted'));
  for (const id of required('now-playing')) {
    const streams = { ...playing.streams };
    delete streams[id];
    assert.deepEqual(keys({ ...media, pages: [{ ...media.pages[0], tiles: [volume, level, { ...playing, streams }] }] }), ['tile.streams'], id);
  }
  const extra = { ...media, pages: [{ ...media.pages[0], tiles: [{ ...volume, streams: { ...volume.streams, title: 'text' } }, level, playing] }] };
  assert.deepEqual(keys(extra), ['tile.streams']);
});

test('input is true only on a bar Tile with a widget action, and schema 1 has no input', () => {
  const media = read('examples/media.json');
  const withVolume = (change) => ({ ...media, pages: [{ ...media.pages[0], tiles: [change(media.pages[0].tiles[0]), ...media.pages[0].tiles.slice(1)] }] });
  assert.deepEqual(keys(withVolume((tile) => tile)), []);
  assert.deepEqual(keys(withVolume(({ input: _input, ...tile }) => tile)), []);
  assert.deepEqual(keys(withVolume((tile) => ({ ...tile, input: false }))), ['tile.input']);
  assert.deepEqual(keys(withVolume((tile) => ({ ...tile, input: 1 }))), ['tile.input']);
  assert.deepEqual(keys(withVolume((tile) => ({ ...tile, action: { type: 'none' } }))), ['tile.input']);
  assert.deepEqual(keys(withVolume(({ render: _render, streams: _streams, ...tile }) => tile)), ['tile.input']);
  assert.deepEqual(keys(withVolume((tile) => ({ ...tile, render: 'graph', streams: { value: 'number' } }))), ['tile.input']);
  assert.deepEqual(keys(withVolume((tile) => ({ ...tile, asset: `sha256:${'a'.repeat(64)}` }))), ['icon.format']);
  assert.deepEqual(keys(withVolume((tile) => ({ ...tile, asset: null }))), []);
});
