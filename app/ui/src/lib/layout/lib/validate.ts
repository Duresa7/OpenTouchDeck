export type Orientation = 'landscape' | 'portrait';
export type Modifier = 'ctrl' | 'shift' | 'alt' | 'win';

export type Action =
  | { type: 'key'; sink?: 'link' | 'bluetooth'; usage: number; mods: Modifier[] }
  | { type: 'mod'; mod: Modifier }
  | { type: 'consumer'; sink?: 'link' | 'bluetooth'; usage: number }
  | { type: 'page'; page: string }
  | { type: 'none' }
  | { type: 'launch'; path: string; args: string[]; cwd: string }
  | { type: 'open'; target: string }
  | { type: 'command'; program: string; args: string[]; cwd: string; shell: boolean }
  | { type: 'widget'; source: string; field: string; account: string };

export type ActionType = Action['type'];

export type Tile = {
  id: string;
  label: string;
  row: number;
  col: number;
  rowSpan: number;
  colSpan: number;
  action: Action;
} & (
  | { render?: undefined; icon: string | null; asset?: string | null; streams?: undefined; input?: undefined }
  | { render: 'gif'; icon: null; asset: string; streams?: undefined; input?: undefined }
  | { render: 'graph'; icon: string | null; asset?: string | null; streams: GraphStreams; input?: undefined }
  | { render: 'bar'; icon: string | null; asset?: null; streams: BarStreams; input?: true }
  | { render: 'now-playing'; icon: string | null; asset?: null; streams: NowPlayingStreams; input?: undefined }
);

export type StreamType = 'number' | 'series' | 'text' | 'image';

/** A graph Tile's one stream: a number appends a sample, a series replaces the history. */
export type GraphStreams = { value: 'number' | 'series' };
/** A bar Tile's level from 0 to 1, and whether it is muted. */
export type BarStreams = { value: 'number'; muted?: 'number' };
/** A now-playing Tile's track, its cover Asset, and 0 nothing playing, 1 paused or 2 playing. */
export type NowPlayingStreams = { title: 'text'; artist: 'text'; cover: 'image'; state: 'number' };

export type Renderer = 'graph' | 'bar' | 'now-playing';
export interface RenderStream {
  types: readonly StreamType[];
  required: boolean;
}

const STREAM_TYPES: readonly StreamType[] = ['number', 'series', 'text', 'image'];
/** The streams each Live tile renderer binds, as schema/live/renderers.json lists them. */
export const RENDER_STREAMS: Record<Renderer, Record<string, RenderStream>> = {
  graph: { value: { types: ['number', 'series'], required: true } },
  bar: { value: { types: ['number'], required: true }, muted: { types: ['number'], required: false } },
  'now-playing': {
    title: { types: ['text'], required: true },
    artist: { types: ['text'], required: true },
    cover: { types: ['image'], required: true },
    state: { types: ['number'], required: true },
  },
};
const isRenderer = (render: unknown): render is Renderer => typeof render === 'string' && Object.hasOwn(RENDER_STREAMS, render);

export interface GridPage {
  id: string;
  title: string;
  kind: 'grid';
  rows: number;
  cols: number;
  tiles: Tile[];
}

interface DashboardPage {
  id: string;
  title: string;
  kind: 'dashboard';
  source: string;
}

export type Page = GridPage | DashboardPage;

export interface Layout {
  schema: 1 | 2;
  name: string;
  orientation: Orientation;
  fullScreen?: boolean;
  pages: Page[];
}

interface LayoutError {
  key: string;
  path: string;
  message: string;
}

const UPLOAD_MAX = 32768;
export const PAGES_MAX = 8;
const TILES_MAX = 64;
export const MODIFIERS: readonly Modifier[] = ['ctrl', 'shift', 'alt', 'win'];
export const ICONS = ['lv:prev', 'lv:play_pause', 'lv:next', 'lv:mute', 'lv:volume_mid', 'lv:volume_max'] as const;
export const ACTION_TYPES: readonly ActionType[] = ['none', 'key', 'mod', 'consumer', 'page', 'launch', 'open', 'command', 'widget'];

const NESTING_MAX = 16;
const fail = (key: string, message: string): LayoutError => ({ key, path: '', message });
const place = (pointer: string) => pointer || '/';
const failAt = (key: string, pointer: string, message: string): LayoutError => ({ key, path: place(pointer), message: `${place(pointer)} ${message}` });
const lone = /\p{Cs}/u;
const wellFormed = (text: string) => !lone.test(text);
const escapePointer = (segment: string | number) =>
  String(segment).replace(/\p{Cs}/gu, '�').replaceAll('~', '~0').replaceAll('/', '~1');
const isObject = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value);

function malformedStrings(root: unknown): LayoutError[] {
  const errors: LayoutError[] = [];
  const pending: [string, unknown][] = [['', root]];
  while (pending.length) {
    const [pointer, value] = pending.pop()!;
    if (typeof value === 'string') {
      if (!wellFormed(value)) errors.push(failAt('string.malformed-unicode', pointer, 'is not well-formed Unicode'));
    } else if (Array.isArray(value)) {
      for (let index = value.length - 1; index >= 0; index--) pending.push([`${pointer}/${index}`, value[index]]);
    } else if (isObject(value)) {
      const members = Object.entries(value);
      for (const [name] of members) {
        if (!wellFormed(name)) {
          errors.push(failAt('string.malformed-unicode', pointer, `has the member name ${JSON.stringify(name)}, which is not well-formed Unicode`));
        }
      }
      for (let index = members.length - 1; index >= 0; index--) pending.push([`${pointer}/${escapePointer(members[index][0])}`, members[index][1]]);
    }
  }
  return errors;
}

interface Bounded {
  max: number;
  tooLong: string;
  blank: string;
}

const NAME: Bounded = { max: 24, tooLong: 'name.too-long', blank: 'name.blank' };
const TITLE: Bounded = { max: 24, tooLong: 'title.too-long', blank: 'title.blank' };
const LABEL: Bounded = { max: 16, tooLong: 'label.too-long', blank: 'label.blank' };
const ID = /^[a-z0-9_]{1,15}$/;
const CONTROL = /[\u0000-\u001f\u007f]/;
const VISIBLE = /[^\p{White_Space}]/u;
const ICON = /^(?:lv:(?:prev|play_pause|next|mute|volume_mid|volume_max)|file:[0-9a-f]{16})$/;

const ACTION_FIELDS: Record<ActionType, string[]> = {
  key: ['type', 'usage', 'mods'],
  mod: ['type', 'mod'],
  consumer: ['type', 'usage'],
  page: ['type', 'page'],
  none: ['type'],
  launch: ['type', 'path', 'args', 'cwd'],
  open: ['type', 'target'],
  command: ['type', 'program', 'args', 'cwd', 'shell'],
  widget: ['type', 'source', 'field', 'account'],
};

const isActionType = (value: unknown): value is ActionType => typeof value === 'string' && Object.hasOwn(ACTION_FIELDS, value);

function structuralErrors(root: unknown): LayoutError[] {
  const errors: LayoutError[] = [];
  const at = (key: string, pointer: string, message: string) => errors.push(failAt(key, pointer, message));
  const wrongType = (pointer: string, expected: string) => at('shape.type', pointer, `must be ${expected} (type)`);

  const object = (value: unknown, pointer: string, fields: string[], optional: string[] = []) => {
    if (!isObject(value)) {
      wrongType(pointer, 'object');
      return undefined;
    }
    for (const field of fields) {
      if (!Object.hasOwn(value, field)) at('shape.missing-field', pointer, `must have required property '${field}' (required) ${JSON.stringify(field)}`);
    }
    for (const name of Object.keys(value)) {
      if (!fields.includes(name) && !optional.includes(name)) at('shape.unknown-field', pointer, `must NOT have additional properties (additionalProperties) ${JSON.stringify(name)}`);
    }
    return value;
  };

  const string = (value: unknown, pointer: string) => {
    if (typeof value !== 'string') {
      wrongType(pointer, 'string');
      return undefined;
    }
    if (CONTROL.test(value)) at('string.control-character', pointer, 'must not contain U+0000 to U+001F or U+007F (pattern)');
    return value;
  };

  const text = (value: unknown, pointer: string) => {
    if (string(value, pointer) === '') at('text.empty', pointer, 'must NOT have fewer than 1 characters (minLength)');
  };

  const bounded = (value: unknown, pointer: string, rules: Bounded) => {
    const content = string(value, pointer);
    if (content === undefined) return;
    if ([...content].length > rules.max) at(rules.tooLong, pointer, `must NOT have more than ${rules.max} characters (maxLength)`);
    if (!VISIBLE.test(content)) at(rules.blank, pointer, 'must hold a character that is not whitespace (pattern)');
  };

  const id = (value: unknown, pointer: string) => {
    if (typeof value !== 'string') return wrongType(pointer, 'string');
    if (!ID.test(value)) at('id.format', pointer, 'must match pattern "^[a-z0-9_]{1,15}$" (pattern)');
  };

  const integer = (value: unknown, pointer: string, min: number, max: number, key: string) => {
    if (typeof value !== 'number') return wrongType(pointer, 'integer');
    if (!Number.isInteger(value)) wrongType(pointer, 'integer');
    if (value < min) at(key, pointer, `must be >= ${min} (minimum)`);
    else if (value > max) at(key, pointer, `must be <= ${max} (maximum)`);
  };

  const modifier = (value: unknown, pointer: string) => {
    if (!MODIFIERS.some((mod) => mod === value)) at('modifier.unknown', pointer, 'must be equal to one of the allowed values (enum)');
  };

  const args = (value: unknown, pointer: string) => {
    if (!Array.isArray(value)) return wrongType(pointer, 'array');
    value.forEach((item, index) => string(item, `${pointer}/${index}`));
  };

  const mods = (value: unknown, pointer: string) => {
    if (!Array.isArray(value)) return wrongType(pointer, 'array');
    const seen = value.map((item) => JSON.stringify(item));
    if (seen.some((item, index) => seen.indexOf(item) !== index)) at('key.mods-duplicate', pointer, 'must NOT have duplicate items (uniqueItems)');
    value.forEach((item, index) => modifier(item, `${pointer}/${index}`));
  };

  const action = (value: unknown, pointer: string) => {
    if (!isObject(value)) return wrongType(pointer, 'object');
    const kind = isActionType(value.type) ? value.type : undefined;
    if (!kind) return at('action.type', pointer, 'must match exactly one schema in oneOf (oneOf)');
    const fields = ACTION_FIELDS[kind];
    object(value, pointer, fields, kind === 'key' || kind === 'consumer' ? ['sink'] : []);
    if ((kind === 'key' || kind === 'consumer') && Object.hasOwn(value, 'sink') && value.sink !== 'link' && value.sink !== 'bluetooth') at('action.sink', `${pointer}/sink`, 'must be link or bluetooth');
    for (const name of fields.slice(1)) {
      if (!Object.hasOwn(value, name)) continue;
      const field = value[name];
      const where = `${pointer}/${name}`;
      if (kind === 'key' && name === 'usage') integer(field, where, 1, 255, 'key.usage-range');
      else if (kind === 'consumer' && name === 'usage') integer(field, where, 1, 65535, 'consumer.usage-range');
      else if (name === 'mods') mods(field, where);
      else if (name === 'mod') modifier(field, where);
      else if (name === 'page') id(field, where);
      else if (name === 'args') args(field, where);
      else if (name === 'cwd' || name === 'account') string(field, where);
      else if (name === 'shell') {
        if (typeof field !== 'boolean') wrongType(where, 'boolean');
      } else text(field, where);
    }
  };

  const tile = (value: unknown, pointer: string) => {
    const tile = object(value, pointer, ['id', 'label', 'row', 'col', 'rowSpan', 'colSpan', 'icon', 'action'], isObject(root) && root.schema === 2 ? ['asset', 'render', 'streams', 'input'] : []);
    if (!tile) return;
    const v2 = isObject(root) && root.schema === 2;
    if (v2 && Object.hasOwn(tile, 'render')) {
      const drawn = tile.render === 'gif' ? typeof tile.asset === 'string' : isRenderer(tile.render) && (tile.render === 'graph' || typeof tile.asset !== 'string');
      if (!drawn) at('icon.format', `${pointer}/render`, 'render must be graph, bar or now-playing, or gif with an Asset reference; bar and now-playing take no Asset');
    }
    if (v2 && Object.hasOwn(tile, 'input') && (tile.input !== true || tile.render !== 'bar' || !isObject(tile.action) || tile.action.type !== 'widget')) {
      at('tile.input', `${pointer}/input`, 'input must be true, on a bar Tile with a widget action');
    }
    if (Object.hasOwn(tile, 'streams')) {
      const streams = tile.streams;
      if (!isObject(streams)) wrongType(`${pointer}/streams`, 'object');
      else if (!Object.values(streams).every((type) => STREAM_TYPES.some((known) => known === type))) at('tile.streams', `${pointer}/streams`, 'stream types must be number, series, text or image');
    }
    if (Object.hasOwn(tile, 'id')) id(tile.id, `${pointer}/id`);
    if (Object.hasOwn(tile, 'label')) bounded(tile.label, `${pointer}/label`, LABEL);
    for (const [name, min, max, key] of [
      ['row', 0, 7, 'tile.position'],
      ['col', 0, 7, 'tile.position'],
      ['rowSpan', 1, 8, 'tile.span'],
      ['colSpan', 1, 8, 'tile.span'],
    ] as const) {
      if (Object.hasOwn(tile, name)) integer(tile[name], `${pointer}/${name}`, min, max, key);
    }
    if (Object.hasOwn(tile, 'icon') && tile.icon !== null) {
      if (typeof tile.icon !== 'string') wrongType(`${pointer}/icon`, 'string,null');
      else if (!ICON.test(tile.icon)) at('icon.format', `${pointer}/icon`, 'must be null, a built-in lv: name, or file: and 16 lowercase hex digits (pattern)');
    }
    if (tile.asset != null && (typeof tile.asset !== 'string' || !/^sha256:[0-9a-f]{64}$/.test(tile.asset) || tile.icon !== null)) at('icon.format', `${pointer}/asset`, 'Asset requires a SHA-256 reference and icon null');
    if (Object.hasOwn(tile, 'action')) action(tile.action, `${pointer}/action`);
  };

  const page = (value: unknown, pointer: string) => {
    if (!isObject(value)) return wrongType(pointer, 'object');
    const fields =
      value.kind === 'grid'
        ? ['id', 'title', 'kind', 'rows', 'cols', 'tiles']
        : value.kind === 'dashboard'
          ? ['id', 'title', 'kind', 'source']
          : undefined;
    if (!fields) return at('page.kind', pointer, 'must match exactly one schema in oneOf (oneOf)');
    object(value, pointer, fields);
    if (Object.hasOwn(value, 'id')) id(value.id, `${pointer}/id`);
    if (Object.hasOwn(value, 'title')) bounded(value.title, `${pointer}/title`, TITLE);
    if (value.kind === 'dashboard') {
      if (Object.hasOwn(value, 'source')) text(value.source, `${pointer}/source`);
      return;
    }
    for (const name of ['rows', 'cols']) {
      if (Object.hasOwn(value, name)) integer(value[name], `${pointer}/${name}`, 1, 8, 'grid.size');
    }
    if (!Object.hasOwn(value, 'tiles')) return;
    const tiles = value.tiles;
    if (!Array.isArray(tiles)) return wrongType(`${pointer}/tiles`, 'array');
    if (tiles.length > TILES_MAX) at('page.tile-count', `${pointer}/tiles`, `must NOT have more than ${TILES_MAX} items (maxItems)`);
    tiles.forEach((item, index) => tile(item, `${pointer}/tiles/${index}`));
  };

  const layout = object(root, '', ['schema', 'name', 'orientation', 'pages'], ['fullScreen']);
  if (!layout) return errors;
  if (Object.hasOwn(layout, 'fullScreen') && typeof layout.fullScreen !== 'boolean') wrongType('/fullScreen', 'boolean');
  if (Object.hasOwn(layout, 'schema') && layout.schema !== 1 && layout.schema !== 2) at('layout.schema-version', '/schema', 'must be equal to constant (const)');
  if (Object.hasOwn(layout, 'name')) bounded(layout.name, '/name', NAME);
  if (Object.hasOwn(layout, 'orientation') && layout.orientation !== 'landscape' && layout.orientation !== 'portrait') {
    at('layout.orientation', '/orientation', 'must be equal to one of the allowed values (enum)');
  }
  if (!Object.hasOwn(layout, 'pages')) return errors;
  const pages = layout.pages;
  if (!Array.isArray(pages)) {
    wrongType('/pages', 'array');
    return errors;
  }
  if (pages.length < 1 || pages.length > PAGES_MAX) at('layout.page-count', '/pages', `must have 1 to ${PAGES_MAX} items, not ${pages.length} (minItems, maxItems)`);
  pages.forEach((item, index) => page(item, `/pages/${index}`));
  return errors;
}

function semanticErrors(layout: Layout): LayoutError[] {
  const errors: LayoutError[] = [];
  const compactBytes = new TextEncoder().encode(JSON.stringify(layout)).length;
  if (compactBytes > UPLOAD_MAX) {
    errors.push(fail('layout.too-large', `layout compact UTF-8 JSON is ${compactBytes} bytes; maximum is ${UPLOAD_MAX} bytes`));
  }
  const ids = new Map<string, string>();
  const pageIds = new Set(layout.pages.map((page) => page.id));
  const addId = (id: string, location: string) => {
    if (ids.has(id)) errors.push(fail('id.duplicate', `duplicate id "${id}" at ${location}; first used at ${ids.get(id)}`));
    else ids.set(id, location);
  };
  for (const [pageIndex, page] of layout.pages.entries()) {
    const pageLocation = `page ${pageIndex} "${page.id}"`;
    addId(page.id, pageLocation);
    if (page.kind === 'dashboard') continue;
    const occupied = new Map<number, string[]>();
    for (const [tileIndex, tile] of page.tiles.entries()) {
      addId(tile.id, `${pageLocation} tile ${tileIndex} "${tile.id}"`);
      if (tile.action.type === 'page' && !pageIds.has(tile.action.page)) {
        errors.push(fail('action.page-missing', `page "${page.id}" tile "${tile.id}" references missing page "${tile.action.page}"`));
      }
      const bindings: Record<string, RenderStream> = isRenderer(tile.render) ? RENDER_STREAMS[tile.render] : {};
      const streams: Record<string, string> = tile.streams ?? {};
      for (const [id, type] of Object.entries(streams)) {
        if (!bindings[id]?.types.some((allowed) => allowed === type)) errors.push(fail('tile.streams', `page "${page.id}" tile "${tile.id}" cannot bind stream "${id}" as ${JSON.stringify(type)}`));
      }
      for (const [id, binding] of Object.entries(bindings)) {
        if (binding.required && !Object.hasOwn(streams, id)) errors.push(fail('tile.streams', `page "${page.id}" tile "${tile.id}" needs stream "${id}"`));
      }
      if (tile.row + tile.rowSpan > page.rows || tile.col + tile.colSpan > page.cols) {
        errors.push(fail('tile.bounds', `page "${page.id}" tile "${tile.id}" exceeds grid bounds ${page.rows}x${page.cols}`));
        continue;
      }
      const overlaps = new Set<string>();
      for (let row = tile.row; row < tile.row + tile.rowSpan; row++) {
        for (let col = tile.col; col < tile.col + tile.colSpan; col++) {
          const cell = row * page.cols + col;
          if (!occupied.has(cell)) occupied.set(cell, []);
          for (const other of occupied.get(cell)!) overlaps.add(other);
          occupied.get(cell)!.push(tile.id);
        }
      }
      for (const other of overlaps) errors.push(fail('tile.overlap', `page "${page.id}" tiles "${other}" and "${tile.id}" overlap`));
    }
  }
  return errors;
}

export function validateLayout(layout: unknown): LayoutError[] {
  const errors = malformedStrings(layout);
  const structural = structuralErrors(layout);
  if (structural.length) return [...errors, ...structural];
  return [...errors, ...semanticErrors(layout as Layout)];
}

const jsonToken = /[ \t\n\r]+|"(?:[^"\\]|\\.)*"|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?|[{}[\]:,]|true|false|null/y;
const integerToken = /^(?:0|-?[1-9]\d*)$/;

function nestsTooDeep(text: string) {
  let depth = 0;
  jsonToken.lastIndex = 0;
  for (let match; (match = jsonToken.exec(text)); ) {
    if (match[0] === '{' || match[0] === '[') {
      if (++depth > NESTING_MAX) return true;
    } else if (match[0] === '}' || match[0] === ']') {
      depth--;
    }
  }
  return false;
}

interface Container {
  pointer: string;
  members?: Set<string>;
  member?: string;
  expectMember?: boolean;
  index?: number;
}

function scanSource(text: string): LayoutError[] {
  const errors: LayoutError[] = [];
  const containers: Container[] = [];
  const valuePointer = () => {
    const top = containers.at(-1);
    if (!top) return '';
    return `${top.pointer}/${top.members ? escapePointer(top.member ?? '') : top.index}`;
  };
  jsonToken.lastIndex = 0;
  for (let match; (match = jsonToken.exec(text)); ) {
    const token = match[0];
    const top = containers.at(-1);
    if (token === '{') {
      containers.push({ pointer: valuePointer(), members: new Set(), expectMember: true });
    } else if (token === '[') {
      containers.push({ pointer: valuePointer(), index: 0 });
    } else if (token === '}' || token === ']') {
      containers.pop();
    } else if (token === ',') {
      if (top?.members) top.expectMember = true;
      else if (top) top.index = (top.index ?? 0) + 1;
    } else if (token[0] === '"' && top?.members && top.expectMember) {
      top.member = String(JSON.parse(token));
      top.expectMember = false;
      if (top.members.has(top.member)) errors.push(failAt('json.duplicate-member', top.pointer, `repeats the member name ${JSON.stringify(top.member)}`));
      top.members.add(top.member);
    } else if (/^-?\d/.test(token) && !integerToken.test(token)) {
      errors.push(failAt('json.integer-form', valuePointer(), `is written as ${token}; numbers must be JSON integers`));
    }
  }
  return errors;
}

export function validateSource(source: Uint8Array | string): LayoutError[] {
  let text: string;
  if (typeof source === 'string') {
    text = source;
  } else {
    try {
      text = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(source);
    } catch {
      return [fail('file.utf8', 'file is not well-formed UTF-8')];
    }
  }
  if (nestsTooDeep(text)) return [fail('file.json', `JSON nests deeper than ${NESTING_MAX} levels`)];
  let layout: unknown;
  try {
    layout = JSON.parse(text);
  } catch (error) {
    return [fail('file.json', error instanceof Error ? error.message : String(error))];
  }
  return [...scanSource(text), ...validateLayout(layout)];
}

export function parseLayout(source: Uint8Array | string): { ok: true; layout: Layout } | { ok: false; errors: LayoutError[] } {
  const errors = validateSource(source);
  if (errors.length) return { ok: false, errors };
  const text = typeof source === 'string' ? source : new TextDecoder().decode(source);
  return { ok: true, layout: JSON.parse(text) as Layout };
}

export const canonical = (layout: Layout) => JSON.stringify(layout);

/** A refused edit names the refused value by JSON pointer, or empty. */
export type Edit = { ok: true; layout: Layout } | { ok: false; reason: string; path: string };

const refuse = (reason: string, path = ''): Edit => ({ ok: false, reason, path });
const copy = (layout: Layout): Layout => JSON.parse(JSON.stringify(layout)) as Layout;

function checked(layout: Layout): Edit {
  const [first] = validateLayout(layout);
  return first ? refuse(`${first.key}: ${first.message}`, first.path) : { ok: true, layout };
}

function gridOf(layout: Layout, page: string): GridPage | string {
  const found = layout.pages.find((candidate) => candidate.id === page);
  if (!found) return `no page ${page}`;
  if (found.kind !== 'grid') return `page ${page} is a dashboard`;
  return found;
}

const covers = (tile: Tile, row: number, col: number) =>
  row >= tile.row && row < tile.row + tile.rowSpan && col >= tile.col && col < tile.col + tile.colSpan;

function fits(grid: GridPage, tile: Tile, skip: number): string | null {
  if (tile.row < 0 || tile.col < 0 || tile.row + tile.rowSpan > grid.rows || tile.col + tile.colSpan > grid.cols) {
    return `outside the ${grid.rows}x${grid.cols} grid`;
  }
  const hit = grid.tiles.find((other, index) => {
    if (index === skip) return false;
    for (let row = tile.row; row < tile.row + tile.rowSpan; row++) {
      for (let col = tile.col; col < tile.col + tile.colSpan; col++) if (covers(other, row, col)) return true;
    }
    return false;
  });
  return hit ? `overlaps ${hit.id}` : null;
}

function editTile(layout: Layout, page: string, id: string, change: (grid: GridPage, index: number) => string | null): Edit {
  const next = copy(layout);
  const grid = gridOf(next, page);
  if (typeof grid === 'string') return refuse(grid);
  const index = grid.tiles.findIndex((tile) => tile.id === id);
  if (index < 0) return refuse(`no tile ${id}`);
  const reason = change(grid, index);
  return reason ? refuse(reason) : checked(next);
}

export function addTile(layout: Layout, page: string, tile: Tile): Edit {
  const next = copy(layout);
  const grid = gridOf(next, page);
  if (typeof grid === 'string') return refuse(grid);
  if (grid.tiles.length >= TILES_MAX) return refuse(`a page holds at most ${TILES_MAX} tiles`);
  const reason = fits(grid, tile, -1);
  if (reason) return refuse(reason);
  grid.tiles.push(JSON.parse(JSON.stringify(tile)) as Tile);
  return checked(next);
}

export function moveTile(layout: Layout, page: string, id: string, row: number, col: number): Edit {
  return editTile(layout, page, id, (grid, index) => {
    const moved = { ...grid.tiles[index], row, col };
    const reason = fits(grid, moved, index);
    if (!reason) grid.tiles[index] = moved;
    return reason;
  });
}

export function resizeTile(layout: Layout, page: string, id: string, rowSpan: number, colSpan: number): Edit {
  return editTile(layout, page, id, (grid, index) => {
    if (rowSpan < 1 || colSpan < 1) return `outside the ${grid.rows}x${grid.cols} grid`;
    const resized = { ...grid.tiles[index], rowSpan, colSpan };
    const reason = fits(grid, resized, index);
    if (!reason) grid.tiles[index] = resized;
    return reason;
  });
}

export function setAction(layout: Layout, page: string, id: string, action: Action): Edit {
  return editTile(layout, page, id, (grid, index) => {
    grid.tiles[index].action = JSON.parse(JSON.stringify(action)) as Action;
    return null;
  });
}

/** Where a graph Tile reads its values: an Integration and one of its streams. */
export interface GraphBinding {
  integration: string;
  stream: string;
}

export type WidgetAction = Extract<Action, { type: 'widget' }>;

/** How a Live tile draws its widget binding: its renderer, the streams it declares, and for a bar whether a drag sets it. */
export type LiveLook =
  | { render: 'graph'; streams: GraphStreams }
  | { render: 'bar'; streams: BarStreams; input?: true }
  | { render: 'now-playing'; streams: NowPlayingStreams };

/** The Tile drawn by a Live tile renderer from the widget binding. A Live tile's action is its binding, and it shows no icon. */
export function liveTile(tile: Tile, action: WidgetAction, look: LiveLook): Tile {
  const { render: _render, streams: _streams, asset: _asset, input: _input, ...rest } = tile;
  const base = { ...rest, icon: null, action: { ...action } };
  switch (look.render) {
    case 'graph':
      return { ...base, render: 'graph', streams: { ...look.streams } };
    case 'bar':
      return look.input ? { ...base, render: 'bar', streams: { ...look.streams }, input: true } : { ...base, render: 'bar', streams: { ...look.streams } };
    case 'now-playing':
      return { ...base, render: 'now-playing', streams: { ...look.streams } };
  }
}

/** The Tile drawn as a graph of the bound stream. A graph Tile's action is its binding, and it shows no icon. */
export function graphTile(tile: Tile, binding: GraphBinding): Tile {
  return liveTile(tile, { type: 'widget', source: binding.integration, field: binding.stream, account: '' }, { render: 'graph', streams: { value: 'number' } });
}

/** Draws the Tile with a Live tile renderer bound through `action`, or, with null, as its label again. */
export function setLive(layout: Layout, page: string, id: string, action: WidgetAction, look: LiveLook | null): Edit {
  return redrawTile(layout, page, id, (tile) => {
    const { render: _render, streams: _streams, asset: _asset, input: _input, ...rest } = tile;
    return look ? liveTile(rest, action, look) : { ...rest, icon: null, action: { ...action } };
  });
}

/** Lets a drag on a bar Tile set its value, or makes the bar read only. Only a bar Tile takes input. */
export function setInput(layout: Layout, page: string, id: string, on: boolean): Edit {
  return editTile(layout, page, id, (grid, index) => {
    const tile = grid.tiles[index];
    if (tile.render !== 'bar') return `tile ${id} is not a bar`;
    const { input: _input, ...rest } = tile;
    grid.tiles[index] = on ? { ...rest, input: true } : rest;
    return null;
  });
}

/** Draws the Tile as a graph of the bound stream, or, with null, as its label again. */
export function setGraph(layout: Layout, page: string, id: string, binding: GraphBinding | null): Edit {
  return redrawTile(layout, page, id, (tile) => {
    const { render: _render, streams: _streams, asset: _asset, input: _input, ...rest } = tile;
    return binding ? graphTile(rest, binding) : { ...rest, icon: null };
  });
}

export function setLabel(layout: Layout, page: string, id: string, label: string): Edit {
  return editTile(layout, page, id, (grid, index) => {
    grid.tiles[index].label = label;
    return null;
  });
}

const needsSchema2 = (layout: Layout) =>
  layout.pages.some((page) => page.kind === 'grid' && page.tiles.some((tile) => tile.asset != null || tile.render !== undefined));

/** Changes how a Tile draws, then sets the Layout schema 2 while any Tile has an Asset or a render, and 1 otherwise. */
function redrawTile(layout: Layout, page: string, id: string, change: (tile: Tile) => Tile): Edit {
  const next = copy(layout);
  const grid = gridOf(next, page);
  if (typeof grid === 'string') return refuse(grid);
  const index = grid.tiles.findIndex((candidate) => candidate.id === id);
  if (index < 0) return refuse(`no tile ${id}`);
  grid.tiles[index] = change(grid.tiles[index]);
  next.schema = needsSchema2(next) ? 2 : 1;
  return checked(next);
}

function iconEdit(layout: Layout, page: string, id: string, icon: string | null, asset: string | null): Edit {
  return redrawTile(layout, page, id, (tile) => {
    const { render: _render, streams: _streams, icon: _icon, asset: _asset, input: _input, ...rest } = tile;
    return asset === null ? { ...rest, icon } : { ...rest, icon: null, asset };
  });
}

export function setIcon(layout: Layout, page: string, id: string, icon: string | null): Edit {
  return iconEdit(layout, page, id, icon, null);
}

export function setAsset(layout: Layout, page: string, id: string, asset: string): Edit {
  return iconEdit(layout, page, id, null, `sha256:${asset}`);
}

/** Plays the prepared GIF Asset on the Tile in place of its icon, or, with null, shows its label again. */
export function setGif(layout: Layout, page: string, id: string, asset: string | null): Edit {
  return redrawTile(layout, page, id, (tile) => {
    const { render: _render, streams: _streams, icon: _icon, asset: _asset, input: _input, ...rest } = tile;
    return asset === null ? { ...rest, icon: null } : { ...rest, icon: null, render: 'gif', asset: `sha256:${asset}` };
  });
}

/** The grid Page that holds the Tile, or null when no Page does. */
export function pageOf(layout: Layout, id: string): string | null {
  return layout.pages.find((page) => page.kind === 'grid' && page.tiles.some((tile) => tile.id === id))?.id ?? null;
}

export function assetOf(tile: Pick<Tile, 'asset'>): string | null {
  return tile.asset?.startsWith('sha256:') ? tile.asset.slice('sha256:'.length) : null;
}

export function removeTile(layout: Layout, page: string, id: string): Edit {
  return editTile(layout, page, id, (grid, index) => {
    grid.tiles.splice(index, 1);
    return null;
  });
}

export function setLayoutName(layout: Layout, name: string): Edit {
  const next = copy(layout);
  next.name = name;
  return checked(next);
}

export function setPageTitle(layout: Layout, page: string, title: string): Edit {
  const next = copy(layout);
  const found = next.pages.find((candidate) => candidate.id === page);
  if (!found) return refuse(`no page ${page}`);
  found.title = title;
  return checked(next);
}

export function setGridSize(layout: Layout, page: string, rows: number, cols: number): Edit {
  const next = copy(layout);
  const grid = gridOf(next, page);
  if (typeof grid === 'string') return refuse(grid);
  if (!Number.isInteger(rows) || !Number.isInteger(cols) || rows < 1 || cols < 1 || rows > 8 || cols > 8) return refuse('grid.size');
  const outside = grid.tiles.find((tile) => tile.row + tile.rowSpan > rows || tile.col + tile.colSpan > cols);
  if (outside) return refuse(`grid.too-small: ${outside.id}`);
  grid.rows = rows;
  grid.cols = cols;
  return checked(next);
}

export function setFullScreen(layout: Layout, on: boolean): Edit {
  const next = copy(layout);
  if (on) {
    const { pages, ...head } = next;
    return checked({ ...head, fullScreen: true, pages });
  }
  delete next.fullScreen;
  return checked(next);
}

export function addPage(layout: Layout, page: Page): Edit {
  if (layout.pages.length >= PAGES_MAX) return refuse(`a layout holds at most ${PAGES_MAX} pages`);
  const next = copy(layout);
  next.pages.push(JSON.parse(JSON.stringify(page)) as Page);
  return checked(next);
}

export function removePage(layout: Layout, page: string): Edit {
  const index = layout.pages.findIndex((candidate) => candidate.id === page);
  if (index < 0) return refuse(`no page ${page}`);
  if (layout.pages.length === 1) return refuse('a layout keeps at least 1 page');
  const next = copy(layout);
  next.pages.splice(index, 1);
  for (const other of next.pages) {
    if (other.kind !== 'grid') continue;
    for (const tile of other.tiles) {
      if (tile.action.type === 'page' && tile.action.page === page) tile.action = { type: 'none' };
    }
  }
  return checked(next);
}

export function freeCell(layout: Layout, page: string): [number, number] | null {
  const grid = gridOf(layout, page);
  if (typeof grid === 'string') return null;
  for (let row = 0; row < grid.rows; row++) {
    for (let col = 0; col < grid.cols; col++) {
      if (!grid.tiles.some((tile) => covers(tile, row, col))) return [row, col];
    }
  }
  return null;
}

export function freshId(layout: Layout, prefix: string): string {
  const taken = new Set<string>();
  for (const page of layout.pages) {
    taken.add(page.id);
    if (page.kind === 'grid') for (const tile of page.tiles) taken.add(tile.id);
  }
  for (let n = 1; ; n++) if (!taken.has(`${prefix}${n}`)) return `${prefix}${n}`;
}

export function newTile(id: string, row: number, col: number): Tile {
  return { id, label: 'Tile', row, col, rowSpan: 1, colSpan: 1, icon: null, action: { type: 'none' } };
}

export function newGridPage(id: string, title: string): GridPage {
  return { id, title, kind: 'grid', rows: 3, cols: 4, tiles: [] };
}

export function newLayout(orientation: Orientation): Layout {
  return { schema: 1, name: 'New Layout', orientation, pages: [newGridPage('page1', 'Page 1')] };
}

const key = (id: string, label: string, row: number, col: number, letter: string, mods: Modifier[]): Tile => ({
  id,
  label,
  row,
  col,
  rowSpan: 1,
  colSpan: 1,
  icon: null,
  action: { type: 'key', usage: 4 + letter.charCodeAt(0) - 65, mods },
});

const media = (id: string, label: string, col: number, row: number, usage: number, icon: string): Tile => ({
  id,
  label,
  row,
  col,
  rowSpan: 1,
  colSpan: 1,
  icon,
  action: { type: 'consumer', usage },
});

export function starterLayout(orientation: Orientation): Layout {
  return {
    schema: 1,
    name: 'Starter',
    orientation,
    pages: [
      {
        id: 'media',
        title: 'Media',
        kind: 'grid',
        rows: 2,
        cols: 3,
        tiles: [
          media('prev', 'Previous', 0, 0, 182, 'lv:prev'),
          media('playpause', 'Play/Pause', 1, 0, 205, 'lv:play_pause'),
          media('next', 'Next', 2, 0, 181, 'lv:next'),
          media('mute', 'Mute', 0, 1, 226, 'lv:mute'),
          media('voldown', 'Volume down', 1, 1, 234, 'lv:volume_mid'),
          media('volup', 'Volume up', 2, 1, 233, 'lv:volume_max'),
        ],
      },
      {
        id: 'shortcuts',
        title: 'Shortcuts',
        kind: 'grid',
        rows: 3,
        cols: 4,
        tiles: [
          key('copy', 'Copy', 0, 0, 'C', ['ctrl']),
          key('paste', 'Paste', 0, 1, 'V', ['ctrl']),
          key('cut', 'Cut', 0, 2, 'X', ['ctrl']),
          key('selectall', 'Select all', 0, 3, 'A', ['ctrl']),
          key('undo', 'Undo', 1, 0, 'Z', ['ctrl']),
          key('redo', 'Redo', 1, 1, 'Y', ['ctrl']),
          key('save', 'Save', 1, 2, 'S', ['ctrl']),
          key('find', 'Find', 1, 3, 'F', ['ctrl']),
          key('snip', 'Screenshot', 2, 0, 'S', ['shift', 'win']),
          key('desktop', 'Show desktop', 2, 1, 'D', ['win']),
        ],
      },
    ],
  };
}

export function counts(layout: Layout) {
  const tiles = layout.pages.reduce((sum, page) => sum + (page.kind === 'grid' ? page.tiles.length : 0), 0);
  return { pages: layout.pages.length, tiles };
}
