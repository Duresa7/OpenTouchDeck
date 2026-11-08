import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'vitest';
import { parseLayout, type GridPage, type Page } from '../layout/model.ts';
import { geometry, screenSize, tileRect } from './geometry.ts';
import { graphPoints } from './graph.ts';
import { BAR, barLayout, nowPlayingLayout } from './live-tiles.ts';
import { buildDots, buildPage, buildTitle, fitLabel, FONTS, missingGlyphs, sampleRows, sampleSeries, sampleWidget, type GraphSample } from './scene.ts';
import { layoutLabel } from './text.ts';
import { panelColor, THEME } from './theme.ts';

const examples = join(import.meta.dirname, '..', '..', '..', '..', '..', 'schema', 'examples');
function load(name: string) {
  const parsed = parseLayout(readFileSync(join(examples, name), 'utf8'));
  if (!parsed.ok) throw new Error(`${name} breaks the Layout rules`);
  return parsed.layout;
}

function gridPage(page: Page): GridPage {
  if (page.kind !== 'grid') throw new Error(`${page.id} is not a grid page`);
  return page;
}
const landscape = geometry(screenSize('landscape'));
const shown = (view: { label: { lines: { letters: { letter: number }[] }[] } }) =>
  view.label.lines.map((line) => line.letters.map((l) => String.fromCodePoint(l.letter)).join(''));

const firmwareUi = join(import.meta.dirname, '..', '..', '..', '..', '..', 'firmware', 'components', 'ui');

test('each Preview font is the font the Firmware draws that text in, at its size and line height', () => {
  const theme = readFileSync(join(firmwareUi, 'ui_theme.c'), 'utf8');
  const fontOf = (role: string) => new RegExp(`\\.font_${role} = &(\\w+),`).exec(theme)?.[1];
  for (const [role, font] of Object.entries(FONTS)) {
    const name = fontOf(role);
    assert.ok(name, role);
    if (name.startsWith('lv_font_montserrat_')) {
      assert.deepEqual([role, font.size, font.lineHeight], ['icon', 48, 52]);
      continue;
    }
    const source = readFileSync(join(firmwareUi, 'fonts', `${name}.c`), 'utf8');
    const metric = (field: string) => Number(new RegExp(`\\.${field} = (\\d+)`).exec(source)?.[1]);
    const size = Number(/Size: (\d+) px/.exec(source)?.[1]);
    assert.deepEqual([font.size, font.lineHeight, font.baseLine], [size, metric('line_height'), metric('base_line')], `${role} is ${name}`);
  }
});

test('a short label keeps the 44 px legend font and sits in the middle of its tile', () => {
  const page = buildPage(load('builtin.json').pages[0], landscape);
  const f1 = page.tiles.find((tile) => tile.id === 'f1')!;
  assert.deepEqual(f1.rect, tileRect({ rows: 4, cols: 4 }, { row: 0, col: 0, rowSpan: 1, colSpan: 1 }, landscape.grid, landscape.gap));
  const [label] = f1.texts;
  assert.equal(label.font, 'legend');
  assert.equal(label.color, THEME.color.text);
  assert.equal(label.y, Math.trunc(f1.rect.h / 2) - Math.trunc(42 / 2));
  assert.equal(label.x, Math.trunc(f1.rect.w / 2) - Math.trunc(label.label.width / 2));
});

test('a label steps down to the 26 px label font, then the 22 px one, then wraps with dots, as fit_label', () => {
  assert.equal(fitLabel('Shift', 200, 100).font, 'legend');
  assert.equal(fitLabel('Ctrl+Shift+F13', 240, 100).font, 'label');
  assert.equal(fitLabel('Ctrl+Shift+F13', 200, 100).font, 'sub');
  assert.equal(fitLabel('Ctrl+Shift+F13', 160, 100).label.lines.length, 2);
  const wrapped = fitLabel('Launch the thing', 54, 97);
  assert.equal(wrapped.font, 'sub');
  assert.equal(wrapped.label.width, 54 - 32);
  assert.ok(wrapped.label.lines.length > 1);
});

test('a label with a letter beyond Basic Latin skips the legend font, which lacks it', () => {
  assert.equal(fitLabel('Cafe', 200, 100).font, 'legend');
  assert.equal(fitLabel('Café', 200, 100).font, 'label');
});

test('a media tile stacks its symbol over a one-line caption', () => {
  const page = buildPage(load('builtin.json').pages[1], landscape);
  const play = page.tiles.find((tile) => tile.id === 'playpause')!;
  const [caption, icon] = play.texts;
  assert.equal(caption.font, 'label');
  assert.equal(caption.label.height, 30);
  assert.equal(caption.label.width, play.rect.w - 32);
  assert.equal(caption.y, Math.trunc(play.rect.h / 2) - 15 + Math.trunc((52 + 14) / 2));
  assert.deepEqual(shown(icon), ['']);
  assert.equal(icon.y, Math.trunc(play.rect.h / 2) - 26 - 22);
  assert.equal(icon.icon, 'lv:play_pause');
  assert.equal(caption.icon, null);
});

test('a widget tile shows its value, its caption below, and a bar as wide as the reading', () => {
  const page = buildPage(load('dev.json').pages[2], landscape, { widget: sampleWidget });
  const five = page.tiles.find((tile) => tile.id === 'five_hour')!;
  const [value, caption] = five.texts;
  assert.deepEqual([value.font, caption.font], ['big', 'label']);
  assert.deepEqual(shown(value), ['42%']);
  assert.deepEqual(shown(caption), ['5h usage']);
  assert.equal(caption.color, THEME.color.caption);
  assert.deepEqual(five.bar, { x: 0, y: five.rect.h - 18, w: Math.trunc(0.42 * five.rect.w + 0.5), h: 6, radius: 3 });
  const bare = buildPage(load('dev.json').pages[2], landscape).tiles[0];
  assert.deepEqual(shown(bare.texts[0]), ['--']);
  assert.equal(bare.bar, null);
});

test('a sample reading draws a bar only where the real one does: none on the requests total for all accounts', () => {
  const widget = (field: string, account: string) =>
    sampleWidget({ id: 't', label: 'Requests 10 min', row: 0, col: 0, rowSpan: 1, colSpan: 1, icon: null, action: { type: 'widget', source: 'cliproxyapi', field, account } });
  assert.deepEqual(widget('recent_10m', ''), { value: '12', bar: -1, caption: 'Requests 10 min' });
  assert.equal(widget('recent_10m', 'codex:Work')?.bar, 0.6);
  assert.equal(widget('five_hour_pct', '')?.bar, 0.42);
});

test('a graph tile plots its samples under its label, in the area the Firmware leaves for it', () => {
  const samples = sampleSeries(20);
  const fresh: GraphSample = { samples, cap: 32, stale: false };
  const page = buildPage(load('graph.json').pages[0], landscape, { graph: () => fresh });
  const cpu = page.tiles.find((tile) => tile.id === 'cpu')!;
  const { pad, top, radius } = THEME.graph;
  const w = cpu.rect.w - 2 * pad;
  const h = cpu.rect.h - top - pad;
  assert.deepEqual(cpu.graph, { x: pad, y: top, w, h, radius, points: graphPoints(samples, 32, w, h), stale: false });
  assert.equal(cpu.graph.points.at(-1)?.x, w - 1);
  assert.deepEqual(cpu.texts.map(shown), [['CPU']]);
  assert.deepEqual([cpu.texts[0].x, cpu.texts[0].opacity, cpu.image, cpu.bar], [pad, 255, null, null]);
});

test('a stale graph keeps its line at 60% with the unavailable marker, and one with no values shows only the marker', () => {
  const stale: GraphSample = { samples: [3, 9, 4], cap: 32, stale: true };
  const page = buildPage(load('graph.json').pages[0], landscape, { graph: (tile) => (tile.id === 'cpu' ? stale : null) });
  const [cpu, net] = page.tiles;
  assert.equal(cpu.graph?.stale, true);
  assert.equal(cpu.graph?.points.length, 3);
  assert.deepEqual(cpu.texts.map(shown), [['CPU'], ['--']]);
  assert.equal(cpu.texts[0].opacity, THEME.staleOpa);
  const marker = cpu.texts[1];
  assert.equal(marker.x + marker.label.width, cpu.rect.w - THEME.graph.pad);
  assert.deepEqual([net.graph?.points, net.graph?.stale], [[], false]);
  assert.deepEqual(net.texts.map(shown), [['Network'], ['--']]);
  const one = buildPage(load('graph.json').pages[0], landscape, { graph: () => ({ samples: [5], cap: 32, stale: false }) });
  assert.deepEqual(one.tiles[0].graph?.points, []);
});

test('a widget tile keeps its caption and value on one line, cut with dots to the tile', () => {
  const usage = gridPage(load('dev.json').pages[2]);
  const page = { ...usage, cols: 4, tiles: [{ ...usage.tiles[0], label: 'Claude 5h usage' }] };
  const long = buildPage(page, landscape, { widget: () => ({ value: '100% of the five hour', caption: 'Claude five-hour usage', bar: -1 }) });
  const [value, caption] = long.tiles[0].texts;
  const maxW = long.tiles[0].rect.w - 32;
  assert.equal(caption.label.lines.length, 1);
  assert.equal(caption.label.width, maxW);
  assert.equal(caption.label.height, 30);
  assert.match(shown(caption)[0], /^Claude .*\.\.\.$/);
  assert.equal(caption.y, Math.trunc(long.tiles[0].rect.h / 2) - 15 + 30);
  assert.equal(value.label.lines.length, 1);
  assert.equal(value.label.width, maxW);
  assert.match(shown(value)[0], /\.\.\.$/);
  const short = buildPage(page, landscape, { widget: () => ({ value: '42%', caption: '5h', bar: -1 }) });
  const [fit, fitCaption] = short.tiles[0].texts;
  assert.deepEqual(shown(fit), ['42%']);
  assert.ok(fit.label.width < maxW);
  assert.equal(fit.x, Math.trunc(short.tiles[0].rect.w / 2) - Math.trunc(fit.label.width / 2));
  assert.deepEqual(shown(fitCaption), ['5h']);
});

test('a dashboard page lays its rows down the grid, 8 rows tall', () => {
  const page = buildPage(load('dev.json').pages[3], landscape, { rows: sampleRows });
  assert.equal(page.rows.length, 3);
  const rowH = Math.trunc((landscape.grid.h - 7 * 8) / 8);
  page.rows.forEach((row, i) => assert.deepEqual(row.rect, { x: 28, y: 76 + i * (rowH + 8), w: 968, h: rowH }));
  assert.equal(page.rows[0].texts[1].x, landscape.dashValueX);
  assert.equal(page.rows[0].bar!.x, landscape.dashBarX);
  assert.equal(buildPage(load('dev.json').pages[3], landscape).rows.length, 0);
});

test('a dashboard row wraps its text only as far as the row holds, the last line cut with dots', () => {
  const dashboard = load('dev.json').pages[3];
  const rows = () => [
    { label: 'claude jordan.k', value: '5h 100% 7d 100%', bar: 1 },
    { label: 'W'.repeat(24), value: '0000000000000000', bar: -1 },
  ];
  const uncut = (value: string, g: typeof landscape) => layoutLabel(value, FONTS.label, { width: g.dashValueX - 2 * THEME.dash.pad });

  const wide = buildPage(dashboard, landscape, { rows });
  const [label, value] = wide.rows[0].texts;
  assert.deepEqual(label.label, uncut('claude jordan.k', landscape));
  assert.equal(label.y, 25 - 15);
  assert.deepEqual(shown(value), ['5h 100% 7d ...']);
  assert.deepEqual([value.font, value.label.width, value.label.height, value.y], ['value', 220, 35, 25 - 17]);
  assert.deepEqual(shown(wide.rows[1].texts[0]), ['WWWWWWWWWWWW...']);

  const portrait = geometry(screenSize('portrait'));
  const tall = buildPage(dashboard, portrait, { rows });
  const [twoLines, twoValues] = tall.rows[0].texts;
  assert.deepEqual(twoLines.label, uncut('claude jordan.k', portrait));
  assert.deepEqual(shown(twoValues), ['5h 100.', '..']);
  assert.deepEqual([twoValues.label.height, twoValues.y], [70, 52 - 35]);
  const cut = tall.rows[1].texts[0];
  assert.equal(uncut('W'.repeat(24), portrait).lines.length, 4);
  assert.deepEqual([cut.label.height, cut.y, cut.label.lines.length], [90, 52 - 45, 3]);
  assert.match(shown(cut)[2], /^W+\.\.\.$/);

  const full = buildPage(dashboard, geometry(screenSize('landscape'), true), { rows });
  assert.deepEqual(shown(full.rows[1].texts[1]), ['0000000000...']);
});

test('the title and dots follow the page, and a full-screen layout draws neither', () => {
  const title = buildTitle('Media', landscape)!;
  assert.deepEqual([title.x, title.y, title.font, title.color], [36, 14, 'title', THEME.color.title]);
  const dots = buildDots(landscape, 2, 1);
  assert.deepEqual(dots[0], { x: 495 - 6, y: 580 - 6, w: 12, h: 12, active: false });
  assert.deepEqual(dots[1], { x: 529 - 6 - 11, y: 574, w: 34, h: 12, active: true });
  const full = geometry(screenSize('landscape'), true);
  assert.equal(buildTitle('Media', full), null);
  assert.deepEqual(buildDots(full, 2, 0), []);
});

test('a title too wide for the screen stays on one line, cut with dots short of the right margin', () => {
  const portrait = geometry(screenSize('portrait'));
  const fits = buildTitle('Usage on every account', portrait)!;
  assert.deepEqual(fits.label.lines, layoutLabel('Usage on every account', FONTS.title).lines);
  assert.deepEqual([fits.x, fits.y, fits.label.height], [36, 14, 30]);
  const wide = buildTitle('W'.repeat(24), portrait)!;
  assert.equal(wide.label.width, 600 - 2 * 36);
  assert.deepEqual(shown(wide), [`${'W'.repeat(17)}...`]);
  assert.deepEqual(shown(buildTitle('W'.repeat(24), landscape)!), ['W'.repeat(24)]);
});

test('characters outside the deck font are listed once', () => {
  assert.deepEqual(missingGlyphs('Café ☕ € ☕'), ['☕', '€']);
  assert.deepEqual(missingGlyphs('Play/Pause 100° Über…'), []);
  assert.deepEqual(missingGlyphs('👩‍👩‍👧‍👦 Call mum'), ['👩‍👩‍👧‍👦']);
});

test('colors widen from RGB565 the way a deck screenshot reads them', () => {
  assert.equal(THEME.color.tile, '#182031');
  assert.equal(THEME.color.bg, '#080810');
  assert.equal(panelColor(72, 160, 255), '#4aa2ff');
});

test('a GIF tile shows its GIF in the middle of the whole tile, with no label or icon', () => {
  const page = buildPage(gridPage(load('gif.json').pages[0]), landscape);
  const gif = page.tiles.find((tile) => tile.gif !== null)!;
  assert.deepEqual(gif.texts, []);
  assert.equal(gif.image, null);
  assert.deepEqual(gif.gif, { id: 'a'.repeat(64), w: gif.rect.w, h: gif.rect.h });
});

const media = () => buildPage(load('media.json').pages[0], landscape, {
  level: (tile) => (tile.id === 'volume' ? { value: 0.64, muted: false, stale: false } : null),
  nowPlaying: () => ({ title: 'Night Drive', artist: 'Tidal Lines', state: 2, stale: false }),
});
const tileOf = (page: ReturnType<typeof media>, id: string) => page.tiles.find((t) => t.id === id)!;

test('a bar Tile draws its speaker, label and level over a track as wide as the reading', () => {
  const volume = tileOf(media(), 'volume');
  const { track, glyph } = barLayout(volume.rect.w, volume.rect.h, true);
  assert.deepEqual(volume.level?.track, { ...track, radius: THEME.level.trackRadius });
  assert.deepEqual(volume.level?.glyph, { ...glyph, muted: false });
  assert.equal(volume.level?.fill, Math.round(0.64 * track.w));
  assert.equal(volume.level?.value.text, '64');
  assert.equal(volume.level?.label.text, 'Volume');
  assert.equal(volume.level?.label.x, glyph.x + glyph.w + THEME.level.gap);
  assert.equal(volume.level?.value.x + volume.level!.value.w, volume.rect.w - BAR.pad);
  assert.deepEqual([volume.level?.stale, volume.level?.muted, volume.level?.input], [false, false, true]);
  assert.deepEqual([volume.texts, volume.bar, volume.graph, volume.nowPlaying], [[], null, null, null]);
});

test('a muted bar shows the mute glyph and greys its fill, and only a bar that declares muted can show it', () => {
  const muted = buildPage(load('media.json').pages[0], landscape, { level: () => ({ value: 0.3, muted: true, stale: false }) });
  assert.deepEqual([tileOf(muted, 'volume').level?.muted, tileOf(muted, 'volume').level?.glyph.muted], [true, true]);
  assert.deepEqual([tileOf(muted, 'level').level?.muted, tileOf(muted, 'level').level?.glyph.muted, tileOf(muted, 'level').level?.input], [false, false, false]);
});

test('a bar with no value shows the unavailable marker and an empty track, and a stale one keeps its fill', () => {
  const unset = tileOf(media(), 'level').level;
  assert.deepEqual([unset?.value.text, unset?.fill, unset?.stale], ['--', 0, true]);
  const stale = tileOf(buildPage(load('media.json').pages[0], landscape, { level: () => ({ value: 0.5, muted: false, stale: true }) }), 'volume').level;
  assert.deepEqual([stale?.value.text, stale?.fill, stale?.stale], ['--', Math.round(0.5 * stale!.track.w), true]);
  const over = tileOf(buildPage(load('media.json').pages[0], landscape, { level: () => ({ value: 3, muted: false, stale: false }) }), 'volume').level;
  assert.deepEqual([over?.value.text, over?.fill], ['100', over?.track.w]);
});

test('a now-playing Tile draws its cover box, title and artist in the text column, and the controls in their zones', () => {
  const playing = tileOf(media(), 'playing');
  const layout = nowPlayingLayout(playing.rect.w, playing.rect.h);
  const view = playing.nowPlaying!;
  assert.deepEqual(view.art, layout.art);
  assert.deepEqual([view.title.text, view.title.x, view.title.w, view.title.idle], ['Night Drive', layout.text.x, layout.text.w, false]);
  assert.deepEqual([view.artist?.text, view.artist?.x, view.artist?.w], ['Tidal Lines', layout.text.x, layout.text.w]);
  assert.ok(view.artist!.y >= view.title.y + view.title.h);
  assert.ok(view.artist!.y + view.artist!.h <= layout.parts[0].y);
  assert.deepEqual(view.controls, layout.parts.map((zone, i) => ({ ...zone, glyph: (['prev', 'pause', 'next'] as const)[i] })));
  const paused = buildPage(load('media.json').pages[0], landscape, { nowPlaying: () => ({ title: 'A', artist: 'B', state: 1, stale: false }) });
  assert.deepEqual(tileOf(paused, 'playing').nowPlaying?.controls.map((c) => c.glyph), ['prev', 'play', 'next']);
});

test('a now-playing Tile shows the title and artist as the Companion fits them for the Device', () => {
  const page = buildPage(load('media.json').pages[0], landscape, {
    nowPlaying: () => ({ title: '\u0141o\u0301dz\u0301\tLive', artist: '\u200bSi\u0219  Band\u2009', state: 2, stale: false }),
  });
  const view = tileOf(page, 'playing').nowPlaying!;
  assert.deepEqual([view.title.text, view.artist?.text], ['L\u00f3dz Live', 'Sis Band']);
});

test('with nothing playing a now-playing Tile says so, with no artist and no controls', () => {
  const idle = buildPage(load('media.json').pages[0], landscape, { nowPlaying: () => ({ title: 'ignored', artist: 'ignored', state: 0, stale: false }) });
  const view = tileOf(idle, 'playing').nowPlaying!;
  assert.deepEqual([view.title.text, view.title.idle, view.artist, view.controls, view.idle], ['Nothing playing', true, null, [], true]);
  assert.deepEqual(view.art, nowPlayingLayout(tileOf(idle, 'playing').rect.w, tileOf(idle, 'playing').rect.h).art);
});

test('a now-playing Tile too small for the controls row draws none, and one with no values shows the unavailable marker', () => {
  const small: Page = { ...gridPage(load('media.json').pages[0]), rows: 8, cols: 4 };
  const page = buildPage(small, landscape, { nowPlaying: () => ({ title: 'Night Drive', artist: 'Tidal Lines', state: 2, stale: false }) });
  const playing = tileOf(page, 'playing');
  assert.equal(nowPlayingLayout(playing.rect.w, playing.rect.h).controls, false);
  assert.deepEqual(playing.nowPlaying?.controls, []);
  const unset = tileOf(buildPage(load('media.json').pages[0], landscape), 'playing').nowPlaying!;
  assert.deepEqual([unset.title.text, unset.artist, unset.controls, unset.stale], ['--', null, [], true]);
});
