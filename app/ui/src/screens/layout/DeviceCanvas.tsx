import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent as ReactPointerEvent } from 'react';
import clsx from 'clsx';
import { Plus } from 'lucide-react';
import { cellAt, geometry, PANEL_H, PANEL_W, screenSize, tileFits, tileRect, type Geometry, type Rect, type Span } from '../../lib/deck/geometry.ts';
import { easeOut, goTo, move, nearestPage, pager, press, release, restX, type Bounce } from '../../lib/deck/pager.ts';
import { buildDots, buildPage, buildTitle, sampleScene, type PageView, type SceneOptions } from '../../lib/deck/scene.ts';
import { THEME } from '../../lib/deck/theme.ts';
import type { EditorStore, Target } from '../../lib/editor/store.ts';
import type { GridPage, Layout, Tile } from '../../lib/layout/model.ts';
import { openDetails } from '../../state/ui.ts';
import { SceneRows, SceneText, TileFace } from './TileScene.tsx';

type ModLook = 'one-shot' | 'locked';

const ARROW_STEPS = new Map<string, readonly [number, number]>([
  ['ArrowUp', [-1, 0]],
  ['ArrowDown', [1, 0]],
  ['ArrowLeft', [0, -1]],
  ['ArrowRight', [0, 1]],
]);

function useStateRef<T>(initial: T): [T, { readonly current: T }, (next: T) => void] {
  const [value, setValue] = useState(initial);
  const ref = useRef(initial);
  const set = (next: T) => {
    ref.current = next;
    setValue(next);
  };
  return [value, ref, set];
}

interface Drag {
  tile: string;
  mode: 'move' | 'resize';
  pointer: number;
  clientX0: number;
  clientY0: number;
  offsetRow: number;
  offsetCol: number;
  span: Span;
  moved: boolean;
}

export interface DeviceCanvasProps {
  store: EditorStore;
  layout: Layout;
  pageId: string;
  mode: 'edit' | 'preview';
  selected: string | null;
  adding: Target | null;
  long: number;
  actualSize: boolean;
  firmwareBadge?: (tile: Tile) => string | null;
  /** How graph Tiles look: the samples the Device keeps, and whether its values are stale because it is offline. */
  live?: { series: number; stale: boolean };
}

function readRatio(name: string, fallback: number) {
  if (typeof window === 'undefined') return fallback;
  const value = Number.parseFloat(getComputedStyle(document.documentElement).getPropertyValue(name));
  return Number.isFinite(value) && value > 0 ? value : fallback;
}

export function deviceAspect(): number {
  const pad = readRatio('--bezel-pad', 0.034);
  return (1 - 2 * pad) * (PANEL_H / PANEL_W) + 2 * pad;
}

const SERIES = 32;

export function DeviceCanvas({ store, layout, pageId, mode, selected, adding, long, actualSize, firmwareBadge, live }: DeviceCanvasProps) {
  const editing = mode === 'edit';
  const g: Geometry = useMemo(() => geometry(screenSize(layout.orientation), layout.fullScreen === true), [layout.orientation, layout.fullScreen]);
  const index = Math.max(0, layout.pages.findIndex((p) => p.id === pageId));
  const current = layout.pages[index];
  const series = live?.series || SERIES;
  const stale = live?.stale ?? false;
  const options: SceneOptions = useMemo(() => sampleScene(series, stale), [series, stale]);
  const views: PageView[] = useMemo(
    () => (editing ? [buildPage(current, g, options)] : layout.pages.map((p) => buildPage(p, g, options))),
    [editing, current, layout.pages, g, options],
  );
  const built = useRef(0);
  built.current = performance.now();
  useLayoutEffect(() => {
    performance.measure?.('page-render', { start: built.current });
  }, [views]);

  const ratios = useMemo(() => ({ pad: readRatio('--bezel-pad', 0.034), radius: readRatio('--bezel-radius', 0.049), glass: readRatio('--bezel-screen-radius', 0.012) }), []);
  const screenLong = Math.max(g.screen.w, g.screen.h);
  const pad = Math.round(long * ratios.pad);
  const scale = actualSize ? 1 / (globalThis.devicePixelRatio || 1) : (long - 2 * pad) / screenLong;
  const deviceLong = actualSize ? screenLong * scale + 2 * pad : long;

  const screenRef = useRef<HTMLDivElement>(null);
  const trackRef = useRef<HTMLDivElement>(null);
  const deckPager = useRef(pager({ pageW: g.screen.w, slop: g.slop, swipe: g.swipe }, layout.pages.length, index));
  const trackX = useRef(-index * g.screen.w);
  const frame = useRef(0);
  const animating = useRef(false);
  const [shown, setShown] = useState(index);
  const [titleIndex, setTitleIndex] = useState(index);
  const [pressed, setPressed] = useState<string | null>(null);
  const [mods, setMods] = useState<Record<string, ModLook>>({});
  const lastModTap = useRef<Record<string, number>>({});
  const reduced = typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;

  const paint = (x: number) => {
    trackX.current = x;
    if (trackRef.current) trackRef.current.style.transform = `translateX(${x}px)`;
    setTitleIndex(nearestPage(x, g.screen.w, layout.pages.length));
  };

  const stageRef = useRef<HTMLDivElement>(null);
  const sliding = (on: boolean) => {
    animating.current = on;
    stageRef.current?.toggleAttribute('data-sliding', on);
  };

  const stop = () => {
    cancelAnimationFrame(frame.current);
    sliding(false);
  };

  function run(from: number, to: number, ms: number, curve: (t: number) => number, done?: () => void) {
    stop();
    if (reduced || ms <= 0) {
      paint(to);
      done?.();
      return;
    }
    sliding(true);
    const start = performance.now();
    const step = (now: number) => {
      const t = Math.min(1, (now - start) / ms);
      paint(Math.round(from + (to - from) * curve(t)));
      if (t < 1) frame.current = requestAnimationFrame(step);
      else {
        sliding(false);
        done?.();
      }
    };
    frame.current = requestAnimationFrame(step);
  }

  const slide = (from: number, to: number, instant = false) => (instant || from === to ? (stop(), paint(to)) : run(from, to, THEME.anim.settleMs, easeOut));

  function bounce(direction: Bounce) {
    const rest = restX(deckPager.current);
    const out = rest + (direction === 'left' ? -1 : 1) * g.bouncePx;
    run(rest, out, THEME.anim.bounceOutMs, easeOut, () => run(out, rest, THEME.anim.bounceBackMs, (t) => 1 - easeOut(1 - t)));
  }

  function showPage(to: number, animate = true) {
    const p = deckPager.current;
    if (to !== shown) setMods((m) => Object.fromEntries(Object.entries(m).filter(([, look]) => look === 'locked')));
    const from = trackX.current;
    goTo(p, to);
    setShown(to);
    slide(from, restX(p), !animate);
    const id = layout.pages[to]?.id;
    if (id && id !== pageId) store.selectPage(id);
  }

  useLayoutEffect(() => {
    const p = deckPager.current;
    Object.assign(p, { pageW: g.screen.w, slop: g.slop, swipe: g.swipe, count: Math.max(1, layout.pages.length) });
    if (editing) {
      stop();
      p.index = index;
      setShown(index);
      setTitleIndex(index);
      trackX.current = 0;
      if (trackRef.current) trackRef.current.style.transform = '';
      return;
    }
    if (p.index !== index) {
      goTo(p, index);
      setShown(index);
      stop();
      paint(restX(p));
    } else if (!animating.current && trackX.current !== restX(p)) {
      paint(restX(p));
    }
  }, [editing, index, g, layout.pages.length]);

  useEffect(() => stop, []);

  const toScreen = (event: { clientX: number; clientY: number }) => {
    const box = screenRef.current!.getBoundingClientRect();
    return { x: (event.clientX - box.left) / scale, y: (event.clientY - box.top) / scale };
  };

  function previewDown(event: ReactPointerEvent) {
    if (editing || event.button !== 0) return;
    const caught = animating.current;
    if (caught) slide(trackX.current, restX(deckPager.current), true);
    const point = toScreen(event);
    press(deckPager.current, Math.trunc(point.x), Math.trunc(point.y));
    const box = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-testid="tile"]') : null;
    const tapped = caught ? null : (box?.dataset.id ?? null);
    setPressed(tapped);
    const onMove = (e: PointerEvent) => {
      const at = toScreen(e);
      const before = deckPager.current.press;
      const m = move(deckPager.current, at.x, at.y);
      if (before === 'tap' && m.press !== 'tap') setPressed(null);
      if (m.changed) showPage(m.to);
      else if (m.bounce) bounce(m.bounce);
    };
    const onUp = (e: PointerEvent) => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onUp);
      const tap = release(deckPager.current) === 'tap';
      setPressed(null);
      if (tap && tapped && e.type === 'pointerup') tapTile(tapped);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onUp);
  }

  function tapTile(id: string) {
    const source = layout.pages[deckPager.current.index];
    if (source?.kind !== 'grid') return;
    const tile = source.tiles.find((t) => t.id === id);
    if (!tile) return;
    const action = tile.action;
    if (action.type === 'page') {
      const to = layout.pages.findIndex((p) => p.id === action.page);
      if (to >= 0 && to !== deckPager.current.index) showPage(to);
    } else if (action.type === 'mod') {
      const now = performance.now();
      const quick = now - (lastModTap.current[action.mod] ?? -Infinity) <= 300;
      lastModTap.current = { ...lastModTap.current, [action.mod]: now };
      setMods((m) => {
        const look = m[action.mod];
        const next = look === undefined ? 'one-shot' : look === 'one-shot' && quick ? 'locked' : undefined;
        const rest = Object.fromEntries(Object.entries(m).filter(([k]) => k !== action.mod));
        return next ? { ...rest, [action.mod]: next } : rest;
      });
    } else if (action.type === 'key' || action.type === 'consumer') {
      setMods((m) => Object.fromEntries(Object.entries(m).filter(([, look]) => look === 'locked')));
    }
  }

  const [drag, dragRef, setDrag] = useStateRef<Drag | null>(null);
  const [target, targetRef, setTarget] = useStateRef<Span | null>(null);
  const tileEls = useRef(new Map<string, HTMLElement>());
  const settle = useRef<{ id: string; x: number; y: number } | null>(null);
  /** Whether the last press on a Tile moved or resized it, so the click that follows doesn't open the Details panel. */
  const dragged = useRef(false);
  const grid = current.kind === 'grid' ? current : null;
  const rectOf = (tile: Span) => (grid ? tileRect(grid, tile, g.grid, g.gap) : { x: 0, y: 0, w: 0, h: 0 });

  function grab(event: ReactPointerEvent<HTMLElement>, tile: Tile, how: Drag['mode']) {
    if (!editing || event.button !== 0 || !grid) return;
    event.stopPropagation();
    dragged.current = false;
    const point = toScreen(event);
    const [row, col] = cellAt(grid, g.grid, g.gap, point.x, point.y);
    event.currentTarget.setPointerCapture?.(event.pointerId);
    store.select(tile.id);
    setDrag({ tile: tile.id, mode: how, pointer: event.pointerId, clientX0: event.clientX, clientY0: event.clientY, offsetRow: row - tile.row, offsetCol: col - tile.col, span: tile, moved: false });
    setTarget(null);
  }

  function follow(event: ReactPointerEvent) {
    const now = dragRef.current;
    if (!now || event.pointerId !== now.pointer || !grid) return;
    event.stopPropagation();
    const dx = event.clientX - now.clientX0;
    const dy = event.clientY - now.clientY0;
    if (!now.moved && Math.hypot(dx, dy) < 4) return;
    if (!now.moved) setDrag({ ...now, moved: true });
    const point = toScreen(event);
    const [row, col] = cellAt(grid, g.grid, g.gap, point.x, point.y);
    const tile = grid.tiles.find((t) => t.id === now.tile);
    if (!tile) return;
    let next: Span;
    if (now.mode === 'move') {
      next = { row: row - now.offsetRow, col: col - now.offsetCol, rowSpan: tile.rowSpan, colSpan: tile.colSpan };
      const el = tileEls.current.get(tile.id);
      if (el) el.style.transform = `translate(${dx / scale}px, ${dy / scale}px)`;
    } else {
      next = { row: tile.row, col: tile.col, rowSpan: Math.max(1, row - tile.row + 1), colSpan: Math.max(1, col - tile.col + 1) };
    }
    const was = targetRef.current;
    if (!was || next.row !== was.row || next.col !== was.col || next.rowSpan !== was.rowSpan || next.colSpan !== was.colSpan) setTarget(next);
  }

  function drop(event: ReactPointerEvent) {
    const done = dragRef.current;
    if (!done || event.pointerId !== done.pointer) return;
    event.stopPropagation();
    const to = targetRef.current;
    dragged.current = done.moved;
    setDrag(null);
    setTarget(null);
    if (!done.moved || !to) {
      const el = tileEls.current.get(done.tile);
      if (el) el.style.transform = '';
      return;
    }
    if (done.mode === 'move') {
      const origin = rectOf(done.span);
      settle.current = { id: done.tile, x: origin.x + (event.clientX - done.clientX0) / scale, y: origin.y + (event.clientY - done.clientY0) / scale };
      if (to.row !== done.span.row || to.col !== done.span.col) store.moveTile(done.tile, to.row, to.col);
    } else if (to.rowSpan !== done.span.rowSpan || to.colSpan !== done.span.colSpan) {
      store.resizeTile(done.tile, to.rowSpan, to.colSpan);
    }
  }

  useLayoutEffect(() => {
    const pending = settle.current;
    if (!pending || drag) return;
    settle.current = null;
    const el = tileEls.current.get(pending.id);
    const tile = grid?.tiles.find((t) => t.id === pending.id);
    if (!el || !tile) return;
    const rect = rectOf(tile);
    el.style.transition = 'none';
    el.style.transform = `translate(${pending.x - rect.x}px, ${pending.y - rect.y}px)`;
    void el.offsetWidth;
    el.style.transition = '';
    el.style.transform = '';
  });

  function tileKeys(event: KeyboardEvent, tile: Tile) {
    if (!editing) {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        tapTile(tile.id);
      }
      return;
    }
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      store.select(tile.id);
      openDetails();
      return;
    }
    if (event.key === 'Escape') {
      event.stopPropagation();
      store.select(null);
      return;
    }
    if (event.key === 'Delete' || event.key === 'Backspace') {
      event.preventDefault();
      event.stopPropagation();
      store.clearTile(tile.id);
      return;
    }
    const step = ARROW_STEPS.get(event.key);
    if (!step) return;
    event.preventDefault();
    event.stopPropagation();
    store.select(tile.id);
    if (event.shiftKey) store.resizeTile(tile.id, tile.rowSpan + step[0], tile.colSpan + step[1]);
    else store.moveTile(tile.id, tile.row + step[0], tile.col + step[1]);
  }

  const free = useMemo(() => {
    if (!editing || !grid) return [];
    const cells: { row: number; col: number; rect: Rect }[] = [];
    for (let row = 0; row < grid.rows; row++) {
      for (let col = 0; col < grid.cols; col++) {
        const taken = grid.tiles.some((t) => row >= t.row && row < t.row + t.rowSpan && col >= t.col && col < t.col + t.colSpan);
        if (!taken) cells.push({ row, col, rect: tileRect(grid, { row, col, rowSpan: 1, colSpan: 1 }, g.grid, g.gap) });
      }
    }
    return cells;
  }, [editing, grid, g]);

  const ghost = useMemo(() => {
    if (!drag?.moved || !target || !grid) return null;
    if (!tileFits(grid, target)) return null;
    const blocked = grid.tiles.some(
      (o) => o.id !== drag.tile && target.row < o.row + o.rowSpan && o.row < target.row + target.rowSpan && target.col < o.col + o.colSpan && o.col < target.col + target.colSpan,
    );
    return { rect: tileRect(grid, target, g.grid, g.gap), blocked };
  }, [drag, target, grid, g]);

  const title = buildTitle(layout.pages[editing ? index : titleIndex]?.title ?? '', g);
  const dots = buildDots(g, layout.pages.length, editing ? index : shown);
  const visible = layout.pages[editing ? index : shown];
  const landscape = g.screen.w >= g.screen.h;
  const screenStyle: CSSProperties = { width: g.screen.w, height: g.screen.h, scale: String(scale), '--s': scale };

  function renderPage(view: PageView, i: number) {
    const source = editing ? current : layout.pages[i];
    const here = editing || i === shown;
    const offset = editing ? 0 : i * g.screen.w;
    if (view.kind !== 'grid' || source.kind !== 'grid') {
      return (
        <div key={view.id} className="scene-page" style={{ left: offset }} aria-hidden={!here} data-testid={here ? 'dashboard-page' : undefined}>
          <SceneRows rows={view.rows} />
        </div>
      );
    }
    const page: GridPage = source;
    return (
      <div
        key={view.id}
        className="scene-page"
        style={{ left: offset }}
        aria-hidden={!here}
        role="group"
        aria-label={`${view.title} Tiles`}
        data-testid={here ? 'grid' : undefined}
        data-page={view.id}
        data-rows={page.rows}
        data-cols={page.cols}
        onPointerDown={(event) => {
          if (editing && event.target === event.currentTarget) store.select(null);
        }}
      >
        {editing &&
          free.map((cell) => {
            const isTarget = adding !== null && adding !== 'new-page' && adding.row === cell.row && adding.col === cell.col;
            return (
              <button
                key={`${cell.row}:${cell.col}`}
                type="button"
                className={clsx('cell-empty', isTarget && 'is-target')}
                style={{ left: cell.rect.x, top: cell.rect.y, width: cell.rect.w, height: cell.rect.h }}
                tabIndex={-1}
                aria-label={`Add Tile at row ${cell.row + 1}, column ${cell.col + 1}`}
                aria-pressed={isTarget}
                data-testid="empty-cell"
                data-row={cell.row}
                data-col={cell.col}
                onClick={() => {
                  store.openAdd({ row: cell.row, col: cell.col });
                  openDetails();
                }}
              >
                <Plus size={44} strokeWidth={2.4} aria-hidden />
              </button>
            );
          })}
        {view.tiles.map((tv) => {
          const tile = page.tiles.find((t) => t.id === tv.id)!;
          const mod = tile.action.type === 'mod' ? mods[tile.action.mod] : undefined;
          const isSelected = editing && selected === tile.id;
          const badge = firmwareBadge?.(tile) ?? null;
          return (
            <div
              key={tile.id}
              ref={(el) => {
                if (el) tileEls.current.set(tile.id, el);
                else tileEls.current.delete(tile.id);
              }}
              className={clsx(
                'tile',
                isSelected && 'is-selected',
                !editing && here && pressed === tile.id && 'is-pressed',
                drag?.moved && drag.tile === tile.id && drag.mode === 'move' && 'is-moving',
                mod === 'one-shot' && 'is-one-shot',
                mod === 'locked' && 'is-locked',
                badge !== null && 'needs-firmware',
              )}
              style={{ left: tv.rect.x, top: tv.rect.y, width: tv.rect.w, height: tv.rect.h }}
              role="button"
              tabIndex={here ? 0 : -1}
              aria-label={tile.label}
              aria-pressed={editing ? isSelected : undefined}
              data-testid="tile"
              data-id={tile.id}
              data-row={tile.row}
              data-col={tile.col}
              data-row-span={tile.rowSpan}
              data-col-span={tile.colSpan}
              data-action={tile.action.type}
              data-needs={badge ?? undefined}
              onPointerDown={(event) => grab(event, tile, 'move')}
              onPointerMove={follow}
              onPointerUp={drop}
              onPointerCancel={drop}
              onClick={() => {
                if (!editing) return;
                store.select(tile.id);
                if (!dragged.current) openDetails();
              }}
              onKeyDown={(event) => tileKeys(event, tile)}
            >
              <TileFace view={tv} label={tile.label} badge={badge} locked={mod === 'locked'} />
              {isSelected && (
                <span
                  className="tile-grip"
                  data-testid="resize-handle"
                  aria-hidden
                  onPointerDown={(event) => grab(event, tile, 'resize')}
                  onPointerMove={follow}
                  onPointerUp={drop}
                  onPointerCancel={drop}
                />
              )}
            </div>
          );
        })}
        {here && ghost && (
          <span
            className={clsx('drag-ghost', ghost.blocked && 'is-blocked')}
            style={{ left: ghost.rect.x, top: ghost.rect.y, width: ghost.rect.w, height: ghost.rect.h }}
            data-testid="drag-ghost"
          />
        )}
      </div>
    );
  }

  const bezelStyle: CSSProperties = {
    '--device-long': `${deviceLong}px`,
    padding: pad,
    borderRadius: Math.round(deviceLong * ratios.radius),
  };

  return (
    <div
      ref={stageRef}
      className={clsx('device-stage', actualSize && 'is-actual')}
      data-testid="deck-preview"
      data-orientation={layout.orientation}
      data-mode={mode}
      data-page={visible?.id}
      data-full-screen={g.fullScreen}
    >
      <div className={clsx('device', landscape ? 'is-landscape' : 'is-portrait')} style={bezelStyle}>
        <div className="device-screen" style={{ width: g.screen.w * scale, height: g.screen.h * scale, borderRadius: Math.round(deviceLong * ratios.glass) }}>
          <div
            ref={screenRef}
            className={clsx('screen', editing ? 'is-editing' : 'is-preview')}
            style={screenStyle}
            data-testid="deck-screen"
            data-scale={scale}
            onPointerDown={previewDown}
          >
            {title && (
              <div data-testid="deck-title">
                <SceneText view={title} />
              </div>
            )}
            <div className="scene-band" style={{ top: 0, width: g.screen.w, height: g.screen.h }}>
              <div ref={trackRef} className="scene-track" style={{ transform: editing ? undefined : `translateX(${trackX.current}px)` }}>
                {views.map(renderPage)}
              </div>
            </div>
            {dots.map((dot, d) => (
              <span key={d} className={clsx('scene-dot', dot.active && 'is-current')} style={{ left: dot.x, top: dot.y, width: dot.w, height: dot.h }} data-testid="deck-dot" />
            ))}
          </div>
          <div className="device-glass" aria-hidden />
        </div>
      </div>
    </div>
  );
}
