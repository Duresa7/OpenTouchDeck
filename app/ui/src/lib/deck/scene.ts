import type { TileAction as Action, Page, Tile } from '../companion/api.gen.ts';
import { assetOf } from '../layout/model.ts';
import { ARCHIVO_MEDIUM_22, ARCHIVO_MEDIUM_26, ARCHIVO_MEDIUM_44, ARCHIVO_NAMEPLATE_26, ARCHIVO_SEMIBOLD_32, ARCHIVO_SEMIBOLD_40, MONTSERRAT_48 } from './fonts.data.ts';
import { dotX, truncDiv, tileRect, type Geometry, type Rect } from './geometry.ts';
import { graphPoints, type GraphPoint } from './graph.ts';
import { fitText, TEXT_BYTES } from './live-text.ts';
import { barLayout, NOW_PLAYING, nowPlayingLayout, type Part } from './live-tiles.ts';
import { font, isMarker, layoutLabel, textSize, codepoints, type Align, type Font, type Label } from './text.ts';
import { SYMBOLS, THEME } from './theme.ts';

/** The Firmware's ui_theme fonts: font_legend, font_label, font_sub, font_value, font_big, font_title and font_icon. */
export type FontRole = 'legend' | 'label' | 'sub' | 'value' | 'big' | 'title' | 'icon';

export const FONTS: Record<FontRole, Font> = {
  legend: font(ARCHIVO_MEDIUM_44),
  label: font(ARCHIVO_MEDIUM_26),
  sub: font(ARCHIVO_MEDIUM_22),
  value: font(ARCHIVO_SEMIBOLD_32),
  big: font(ARCHIVO_SEMIBOLD_40),
  title: font(ARCHIVO_NAMEPLATE_26),
  icon: font(MONTSERRAT_48),
};

const LV_COORD_MAX = (1 << 29) - 1;

export interface TextView {
  x: number;
  y: number;
  font: FontRole;
  color: string;
  opacity: number;
  label: Label;
  icon: string | null;
}

export interface BarView {
  x: number;
  y: number;
  w: number;
  h: number;
  radius: number;
}

export interface GraphView {
  x: number;
  y: number;
  w: number;
  h: number;
  radius: number;
  points: GraphPoint[];
  stale: boolean;
}

/** Text the page draws in a box, cut with dots when it overflows. */
export interface TextBox extends Rect {
  text: string;
}

/** A bar Tile: the speaker glyph, its label and 0 to 100 value, and the track with its fill. */
export interface LevelView {
  glyph: Rect & { muted: boolean };
  label: TextBox;
  value: TextBox;
  track: Rect & { radius: number };
  /** Width of the fill in pixels. */
  fill: number;
  muted: boolean;
  /** No fresh value: drawn at 60% with the unavailable marker, and it takes no drag. */
  stale: boolean;
  input: boolean;
}

export type ControlGlyph = 'prev' | 'play' | 'pause' | 'next';

/** A now-playing Tile: the cover box, the title and artist, and the controls row. */
export interface NowPlayingView {
  art: { x: number; y: number; size: number } | null;
  title: TextBox & { idle: boolean };
  artist: TextBox | null;
  controls: (Rect & { part: Part; glyph: ControlGlyph })[];
  idle: boolean;
  stale: boolean;
}

export interface TileView {
  id: string;
  rect: Rect;
  action: Action['type'];
  texts: TextView[];
  image: { id: string; x: number; y: number; size: number } | null;
  bar: BarView | null;
  graph: GraphView | null;
  /** A GIF Tile's Asset, drawn at its own size in the middle of a box this big. */
  gif: { id: string; w: number; h: number } | null;
  level: LevelView | null;
  nowPlaying: NowPlayingView | null;
}

export interface RowView {
  rect: Rect;
  texts: TextView[];
  bar: BarView | null;
}

export interface PageView {
  id: string;
  title: string;
  kind: Page['kind'];
  tiles: TileView[];
  rows: RowView[];
}

export interface DotView {
  x: number;
  y: number;
  w: number;
  h: number;
  active: boolean;
}

export interface WidgetSample {
  value: string;
  caption?: string;
  bar: number;
  stale?: boolean;
}

export interface RowSample {
  label: string;
  value: string;
  bar: number;
}

/** What a graph Tile shows: its samples, oldest first, how many the Device keeps, and whether they are stale. */
export interface GraphSample {
  samples: readonly number[];
  cap: number;
  stale: boolean;
}

/** What a bar Tile shows: its level from 0 to 1, whether it is muted, and whether the value is stale. */
export interface LevelSample {
  value: number;
  muted: boolean;
  stale: boolean;
}

/** What a now-playing Tile shows: the track, 0 nothing playing, 1 paused or 2 playing, and whether it is stale. */
export interface NowPlayingSample {
  title: string;
  artist: string;
  state: number;
  stale: boolean;
}

export interface SceneOptions {
  widget?: (tile: Tile) => WidgetSample | null;
  graph?: (tile: Tile) => GraphSample | null;
  level?: (tile: Tile) => LevelSample | null;
  nowPlaying?: (tile: Tile) => NowPlayingSample | null;
  rows?: (page: Page) => RowSample[];
  hasImage?: (id: string) => boolean;
}

const centre = (pw: number, ph: number, w: number, h: number, dx = 0, dy = 0) => ({
  x: truncDiv(pw, 2) - truncDiv(w, 2) + dx,
  y: truncDiv(ph, 2) - truncDiv(h, 2) + dy,
});

function text(label: Label, role: FontRole, color: string, at: { x: number; y: number }, opacity = 255): TextView {
  return { x: at.x, y: at.y, font: role, color, opacity, label, icon: null };
}

/** The Firmware's fit_label. */
export function fitLabel(value: string, tileW: number, tileH: number): { label: Label; font: FontRole } {
  const letters = codepoints(value);
  if (letters.every((letter) => FONTS.legend.has(letter))) {
    const legend = textSize(letters, FONTS.legend, LV_COORD_MAX);
    if (legend.w <= tileW && legend.h <= tileH) return { label: layoutLabel(value, FONTS.legend), font: 'legend' };
  }
  const maxW = tileW - 2 * THEME.media.labelInset;
  for (const role of ['label', 'sub'] as const) {
    const measured = textSize(letters, FONTS[role], LV_COORD_MAX);
    if (measured.w <= maxW && measured.h <= tileH) return { label: layoutLabel(value, FONTS[role]), font: role };
  }
  const wrapped = textSize(letters, FONTS.sub, maxW);
  return {
    label: layoutLabel(value, FONTS.sub, { width: maxW, height: wrapped.h > tileH ? tileH : null, align: 'center', dots: true }),
    font: 'sub',
  };
}

const fileId = (icon: string | null) => (icon?.startsWith('file:') ? icon.slice(5) : null);

/** The Firmware's ui_live_build: the label top left, "--" top right while there is no fresh value, and the plot below. */
function buildGraph(view: TileView, tile: Tile, sample: GraphSample | null) {
  const { pad, top, radius } = THEME.graph;
  const { rect } = view;
  const stale = !sample || sample.stale;
  const lineH = FONTS.label.lineHeight;
  const y = truncDiv(top - lineH, 2);
  const label = layoutLabel(tile.label, FONTS.label, { width: rect.w - 2 * pad, height: lineH, dots: true });
  view.texts.push(text(label, 'label', THEME.color.caption, { x: pad, y }, sample?.stale ? THEME.staleOpa : 255));
  if (stale) {
    const marker = layoutLabel('--', FONTS.label);
    view.texts.push(text(marker, 'label', THEME.color.caption, { x: rect.w - pad - marker.width, y }));
  }
  const w = Math.max(rect.w - 2 * pad, 0);
  const h = Math.max(rect.h - top - pad, 0);
  const points = sample ? graphPoints(sample.samples, sample.cap, w, h) : [];
  view.graph = { x: pad, y: top, w, h, radius, points: points.length < 2 ? [] : points, stale: sample?.stale ?? false };
}

const UNAVAILABLE = '--';
export const NOTHING_PLAYING = 'Nothing playing';

function buildLevel(rect: Rect, tile: Tile, sample: LevelSample | null): LevelView {
  const { gap, valueW, lineH, trackRadius } = THEME.level;
  const declared = tile.streams?.muted !== undefined;
  const { glyph, track } = barLayout(rect.w, rect.h, declared);
  const muted = declared && sample !== null && sample.muted;
  const level = sample ? Math.min(Math.max(sample.value, 0), 1) : 0;
  const fresh = sample !== null && !sample.stale;
  const valueX = rect.w - glyph.x - valueW;
  const labelX = glyph.x + glyph.w + gap;
  return {
    glyph: { ...glyph, muted },
    label: { x: labelX, y: glyph.y, w: Math.max(valueX - gap - labelX, 0), h: lineH, text: tile.label },
    value: { x: valueX, y: glyph.y, w: valueW, h: lineH, text: fresh ? String(Math.round(level * 100)) : UNAVAILABLE },
    track: { ...track, radius: trackRadius },
    fill: sample ? Math.round(level * track.w) : 0,
    muted,
    stale: !fresh,
    input: tile.input === true,
  };
}

const GLYPHS: Record<Part, ControlGlyph> = { previous: 'prev', play_pause: 'pause', next: 'next' };

function buildNowPlaying(rect: Rect, sample: NowPlayingSample | null): NowPlayingView {
  const { titleH, artistH } = THEME.nowPlaying;
  const layout = nowPlayingLayout(rect.w, rect.h);
  const idle = sample !== null && sample.state === 0;
  const named = sample !== null && !idle;
  const controls = named && layout.controls ? layout.parts : [];
  const block = named ? titleH + artistH : titleH;
  const top = layout.art?.y ?? NOW_PLAYING.pad;
  const row = controls[0]?.y ?? rect.h;
  const y = !controls.length ? truncDiv(rect.h - block, 2) : top + block <= row ? top : NOW_PLAYING.pad + truncDiv(row - NOW_PLAYING.pad - block, 2);
  const column = { x: layout.text.x, w: layout.text.w };
  return {
    art: layout.art,
    title: { ...column, y: Math.max(y, 0), h: titleH, text: named ? fitText(sample.title, TEXT_BYTES) : idle ? NOTHING_PLAYING : UNAVAILABLE, idle },
    artist: named ? { ...column, y: Math.max(y, 0) + titleH, h: artistH, text: fitText(sample.artist, TEXT_BYTES) } : null,
    controls: controls.map((zone) => ({ ...zone, glyph: zone.part === 'play_pause' && sample?.state === 1 ? 'play' : GLYPHS[zone.part] })),
    idle,
    stale: sample === null || sample.stale,
  };
}

function buildTile(page: Page & { kind: 'grid' }, tile: Tile, g: Geometry, options: SceneOptions): TileView {
  const rect = tileRect(page, tile, g.grid, g.gap);
  const view: TileView = {
    id: tile.id,
    rect,
    action: tile.action.type,
    texts: [],
    image: null,
    bar: null,
    graph: null,
    gif: null,
    level: null,
    nowPlaying: null,
  };
  const { color, media, widget } = THEME;
  const gif = tile.render === 'gif' ? assetOf(tile) : null;
  if (gif !== null) {
    view.gif = { id: gif, w: rect.w, h: rect.h };
    return view;
  }
  if (tile.render === 'graph') {
    buildGraph(view, tile, options.graph?.(tile) ?? null);
    return view;
  }
  if (tile.render === 'bar') {
    view.level = buildLevel(rect, tile, options.level?.(tile) ?? null);
    return view;
  }
  if (tile.render === 'now-playing') {
    view.nowPlaying = buildNowPlaying(rect, options.nowPlaying?.(tile) ?? null);
    return view;
  }
  if (tile.action.type === 'widget') {
    const sample = options.widget?.(tile) ?? null;
    const opacity = sample?.stale ? THEME.staleOpa : 255;
    const maxW = rect.w - 2 * media.labelInset;
    const shown = sample?.value || '--';
    const fits = textSize(codepoints(shown), FONTS.big, LV_COORD_MAX).w <= maxW;
    const valueBox = fits ? {} : { width: maxW, align: 'center' as const };
    const value = layoutLabel(shown, FONTS.big, { ...valueBox, height: FONTS.big.lineHeight, dots: true });
    view.texts.push(text(value, 'big', color.text, centre(rect.w, rect.h, value.width, value.height, 0, -widget.valueDy), opacity));
    const caption = layoutLabel(sample?.caption || tile.label, FONTS.label, { width: maxW, height: FONTS.label.lineHeight, align: 'center', dots: true });
    view.texts.push(text(caption, 'label', color.caption, centre(rect.w, rect.h, caption.width, caption.height, 0, widget.captionDy), opacity));
    const barW = !sample || sample.bar < 0 ? 0 : Math.trunc(Math.min(sample.bar, 1) * rect.w + 0.5);
    if (barW > 0) view.bar = { x: 0, y: rect.h - widget.barBottom - widget.barH, w: barW, h: widget.barH, radius: widget.barRadius };
    return view;
  }
  const glyph = tile.icon ? SYMBOLS[tile.icon] : undefined;
  const image = fileId(tile.icon) ?? assetOf(tile);
  const stacked = glyph !== undefined || (image !== null && (options.hasImage?.(image) ?? true));
  if (!stacked) {
    const fitted = fitLabel(tile.label, rect.w, rect.h);
    view.texts.push(text(fitted.label, fitted.font, color.text, centre(rect.w, rect.h, fitted.label.width, fitted.label.height)));
    return view;
  }
  const captionH = FONTS.label.lineHeight;
  const iconH = glyph !== undefined ? FONTS.icon.lineHeight : 64;
  const caption = layoutLabel(tile.label, FONTS.label, { width: rect.w - 2 * media.labelInset, height: captionH, align: 'center' });
  view.texts.push(text(caption, 'label', color.caption, centre(rect.w, rect.h, caption.width, caption.height, 0, truncDiv(iconH + media.gap, 2))));
  const lift = truncDiv(-(captionH + media.gap), 2);
  if (glyph !== undefined) {
    const icon = layoutLabel(glyph, FONTS.icon);
    view.texts.push({ ...text(icon, 'icon', color.text, centre(rect.w, rect.h, icon.width, icon.height, 0, lift)), icon: tile.icon });
  } else if (image !== null) {
    view.image = { id: image, ...centre(rect.w, rect.h, 64, 64, 0, lift), size: 64 };
  }
  return view;
}

function buildRows(g: Geometry, rows: RowSample[]): RowView[] {
  const { dash, color } = THEME;
  const rowH = truncDiv(g.grid.h - (dash.rowsMax - 1) * dash.rowGap, dash.rowsMax);
  const barMax = g.grid.w - g.dashBarX - dash.pad;
  const cell = (value: string, role: 'label' | 'value', x: number, width: number, tint: string, align: Align = 'left') => {
    const f = FONTS[role];
    const maxH = truncDiv(rowH, f.lineHeight) * f.lineHeight;
    const height = Math.min(textSize(codepoints(value), f, width).h, maxH);
    const label = layoutLabel(value, f, { width, height, align, dots: true });
    return text(label, role, tint, { x, y: truncDiv(rowH, 2) - truncDiv(label.height, 2) });
  };
  return rows.slice(0, dash.rowsMax).map((row, i) => {
    const barW = row.bar < 0 ? 0 : Math.trunc(Math.min(row.bar, 1) * barMax + 0.5);
    return {
      rect: { x: g.grid.x, y: g.grid.y + i * (rowH + dash.rowGap), w: g.grid.w, h: rowH },
      texts: [
        cell(row.label, 'label', dash.pad, g.dashValueX - 2 * dash.pad, color.caption),
        cell(row.value, 'value', g.dashValueX, g.dashBarX - g.dashValueX - dash.pad, color.text),
      ],
      bar: barW > 0 ? { x: g.dashBarX, y: truncDiv(rowH, 2) - truncDiv(dash.barH, 2), w: barW, h: dash.barH, radius: THEME.widget.barRadius } : null,
    };
  });
}

export function buildPage(page: Page, g: Geometry, options: SceneOptions = {}): PageView {
  if (page.kind === 'dashboard') {
    return { id: page.id, title: page.title, kind: page.kind, tiles: [], rows: buildRows(g, options.rows?.(page) ?? []) };
  }
  return { id: page.id, title: page.title, kind: page.kind, tiles: page.tiles.map((tile) => buildTile(page, tile, g, options)), rows: [] };
}

export function buildTitle(title: string, g: Geometry): TextView | null {
  if (g.fullScreen) return null;
  const label = layoutLabel(title, FONTS.title, { width: g.screen.w - 2 * g.titleX, height: FONTS.title.lineHeight, dots: true });
  return text(label, 'title', THEME.color.title, { x: g.titleX, y: g.titleY });
}

export function buildDots(g: Geometry, count: number, current: number): DotView[] {
  if (g.fullScreen) return [];
  return Array.from({ length: count }, (_, i) => {
    const active = i === current;
    const extra = active ? g.dotActiveExtra : 0;
    return {
      x: dotX(g, i, count) - truncDiv(g.dotSize, 2) - extra,
      y: g.dotY - truncDiv(g.dotSize, 2),
      w: g.dotSize + 2 * extra,
      h: g.dotSize,
      active,
    };
  });
}

const GRAPHEMES = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

const drawn = (code: number) => isMarker(code) || FONTS.label.has(code);

/** Each character the Device's font lacks, listed whole, so a family emoji is one entry and not its parts. */
export function missingGlyphs(value: string): string[] {
  const missing = Array.from(GRAPHEMES.segment(value), (part) => part.segment).filter((part) => !codepoints(part).every(drawn));
  return [...new Set(missing)];
}

const FIELD_SAMPLES: Record<string, Omit<WidgetSample, 'caption'>> = {
  five_hour_pct: { value: '42%', bar: 0.42 },
  seven_day_pct: { value: '61%', bar: 0.61 },
  five_hour_reset: { value: '2h 10m', bar: -1 },
  seven_day_reset: { value: '3d 4h', bar: -1 },
  plan: { value: 'pro', bar: -1 },
  status: { value: 'active', bar: -1 },
  success: { value: '128', bar: -1 },
  failed: { value: '3', bar: -1 },
  recent_10m: { value: '12', bar: 0.6 },
  cpu_pct: { value: '23%', bar: 0.23 },
  ram_pct: { value: '58%', bar: 0.58 },
  ram_used_bytes: { value: '9.3 GB', bar: -1 },
  ram_total_bytes: { value: '16 GB', bar: -1 },
  gpu_pct: { value: '12%', bar: 0.12 },
  disk_active_pct: { value: '4%', bar: 0.04 },
  disk_read_bytes_s: { value: '1.2 MB/s', bar: -1 },
  disk_write_bytes_s: { value: '640 KB/s', bar: -1 },
  network_receive_bytes_s: { value: '2.4 MB/s', bar: -1 },
  network_send_bytes_s: { value: '310 KB/s', bar: -1 },
};

/** Fields whose bar compares one account with the busiest, so the reading for all accounts has none. */
const ACCOUNT_BARS: ReadonlySet<string> = new Set(['recent_10m']);

/** A widget Tile's example reading, with a bar only where the real reading has one. */
export function sampleWidget(tile: Tile): WidgetSample | null {
  if (tile.action.type !== 'widget') return null;
  const { field, account } = tile.action;
  const sample = FIELD_SAMPLES[field];
  if (!sample) return { value: '--', caption: tile.label, bar: -1 };
  return { ...sample, bar: account === '' && ACCOUNT_BARS.has(field) ? -1 : sample.bar, caption: tile.label };
}

/** Samples a preview graph draws until the Device shows real ones: a gentle wave between 10 and 90. */
export function sampleSeries(count: number): number[] {
  return Array.from({ length: count }, (_, i) => Math.round((50 + 40 * Math.sin(i / 3)) * 10) / 10);
}

/** The level a preview bar Tile shows until the Device shows real ones. */
export const SAMPLE_LEVEL = { value: 0.64, muted: false } as const;

/** The track a preview now-playing Tile shows until the Device shows real ones. */
export const SAMPLE_TRACK = { title: 'Night Drive', artist: 'Tidal Lines', state: 2 } as const;

/**
 * What a preview draws on every Live tile: example values, a graph of `series` samples, the sample level and track,
 * all greyed out when `stale`. The Layout canvas and the Preset previews both draw from these.
 */
export function sampleScene(series: number, stale: boolean): SceneOptions {
  const samples = sampleSeries(series);
  return {
    widget: sampleWidget,
    rows: sampleRows,
    hasImage: () => true,
    graph: () => ({ samples, cap: series, stale }),
    level: () => ({ ...SAMPLE_LEVEL, stale }),
    nowPlaying: () => ({ ...SAMPLE_TRACK, stale }),
  };
}

export function sampleRows(): RowSample[] {
  return [
    { label: 'codex alex', value: '5h 42% 7d 10%', bar: 0.42 },
    { label: 'claude sam', value: '5h 18% 7d 61%', bar: 0.61 },
    { label: 'gemini-cli jo', value: 'active', bar: 0.3 },
  ];
}
