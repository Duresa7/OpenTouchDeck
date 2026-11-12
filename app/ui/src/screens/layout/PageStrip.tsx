import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import clsx from 'clsx';
import { ContextMenu } from '@base-ui/react/context-menu';
import { Menu } from '@base-ui/react/menu';
import { MoreHorizontal, Plus, Trash2 } from 'lucide-react';
import { geometry, screenSize, type Geometry } from '../../lib/deck/geometry.ts';
import { buildDots, buildPage, buildTitle, sampleScene, type SceneOptions } from '../../lib/deck/scene.ts';
import type { EditorStore } from '../../lib/editor/store.ts';
import { PAGES_MAX, type Layout, type Page } from '../../lib/layout/model.ts';
import { button } from '../../shell/button.ts';
import { SceneRows, SceneText, TileFace } from './TileScene.tsx';
import { Tip } from './Tip.tsx';

export const PAGES_ID = 'editor-pages';
/** A thumbnail's long side, which fills the column beside the Page number. */
const THUMB_LONG = 112;
const SERIES = 32;
/** How long Remove Page waits for its second click, as it did in the Details panel. */
const ARM_MS = 4000;

interface ThumbProps {
  page: Page;
  index: number;
  count: number;
  g: Geometry;
  options: SceneOptions;
}

const sameThumb = (a: ThumbProps, b: ThumbProps) =>
  a.index === b.index && a.count === b.count && a.g === b.g && a.options === b.options && (a.page === b.page || JSON.stringify(a.page) === JSON.stringify(b.page));

/**
 * The Page as the Device draws it, at the Device's shape and a fixed long side: the Page name, its Tiles or Usage
 * rows, and the pager. Every edit makes a new Layout, so it redraws only when its own Page changed.
 */
const PageThumb = memo(function PageThumb({ page, index, count, g, options }: ThumbProps) {
  const view = buildPage(page, g, options);
  const title = buildTitle(page.title, g);
  const dots = buildDots(g, count, index);
  const scale = THUMB_LONG / Math.max(g.screen.w, g.screen.h);
  const box: CSSProperties = { width: g.screen.w * scale, height: g.screen.h * scale };
  const screen: CSSProperties = { width: g.screen.w, height: g.screen.h, scale: String(scale), '--s': scale };
  return (
    <span className="page-thumb" style={box} data-testid="page-thumb" data-orientation={g.screen.w >= g.screen.h ? 'landscape' : 'portrait'} aria-hidden>
      <span className="page-thumb-screen" style={screen}>
        {title && <SceneText view={title} />}
        {page.kind === 'grid' ? (
          view.tiles.map((tv) => (
            <span key={tv.id} className="tile thumb-tile" style={{ left: tv.rect.x, top: tv.rect.y, width: tv.rect.w, height: tv.rect.h }} data-id={tv.id}>
              <TileFace view={tv} label={page.tiles.find((t) => t.id === tv.id)?.label ?? ''} />
            </span>
          ))
        ) : (
          <SceneRows rows={view.rows} />
        )}
        {dots.map((dot, d) => (
          <span key={d} className={clsx('scene-dot', dot.active && 'is-current')} style={{ left: dot.x, top: dot.y, width: dot.w, height: dot.h }} />
        ))}
      </span>
    </span>
  );
}, sameThumb);

/** Asks once more before it removes, in words, as Remove Page always has. Closing the menu disarms it. */
function RemovePageItem({ page, last, onRemove }: { page: Page; last: boolean; onRemove(): void }) {
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    const timer = setTimeout(() => setArmed(false), ARM_MS);
    return () => clearTimeout(timer);
  }, [armed]);
  return (
    <Menu.Item
      className={clsx('menu-item', 'is-danger', armed && 'is-armed')}
      disabled={last}
      closeOnClick={armed}
      onClick={() => {
        if (armed) onRemove();
        else setArmed(true);
      }}
      data-testid="page-remove"
    >
      <span className="menu-name">
        <Trash2 size={16} strokeWidth={1.9} aria-hidden />
        {armed ? `Remove “${page.title}”?` : 'Remove Page'}
      </span>
      {last && <span className="menu-line">A Layout needs at least one Page</span>}
      {armed && <span className="menu-line">Click again to remove</span>}
    </Menu.Item>
  );
}

interface EntryProps {
  store: EditorStore;
  page: Page;
  onRemove(): void;
  index: number;
  count: number;
  current: boolean;
  g: Geometry;
  options: SceneOptions;
}

/** One Page: its number, thumbnail and name. Right-click it, or use the button that shows on hover and focus, for its menu. */
function PageEntry({ store, page, onRemove, index, count, current, g, options }: EntryProps) {
  const label = `Options for “${page.title}”`;
  const items = <RemovePageItem page={page} last={count <= 1} onRemove={onRemove} />;
  return (
    <li className="page-entry" data-page={page.id}>
      <ContextMenu.Root>
        <ContextMenu.Trigger
          render={
            <button
              type="button"
              className="page-entry-button"
              aria-current={current ? 'true' : undefined}
              onClick={() => store.selectPage(page.id)}
              title={page.title}
              data-testid="page-entry"
              data-page={page.id}
            />
          }
        >
          <span className="page-entry-number tabular" aria-hidden>
            {index + 1}
          </span>
          <PageThumb page={page} index={index} count={count} g={g} options={options} />
          <span className="page-entry-name">
            <span className="page-entry-title" data-testid="page-entry-title">
              {page.title}
            </span>
            {page.kind === 'dashboard' && <span className="page-tag">Usage</span>}
          </span>
        </ContextMenu.Trigger>
        <ContextMenu.Portal>
          <ContextMenu.Positioner className="menu-positioner">
            <ContextMenu.Popup className="menu page-menu" aria-label={label}>
              {items}
            </ContextMenu.Popup>
          </ContextMenu.Positioner>
        </ContextMenu.Portal>
      </ContextMenu.Root>
      <Menu.Root>
        <Tip label="More">
          <Menu.Trigger className={clsx(button({ kind: 'icon' }), 'page-entry-menu')} aria-label={label} data-testid="page-menu">
            <MoreHorizontal size={16} strokeWidth={1.9} aria-hidden />
          </Menu.Trigger>
        </Tip>
        <Menu.Portal>
          <Menu.Positioner className="menu-positioner" side="bottom" align="start" sideOffset={6}>
            <Menu.Popup className="menu page-menu" aria-label={label}>
              {items}
            </Menu.Popup>
          </Menu.Positioner>
        </Menu.Portal>
      </Menu.Root>
    </li>
  );
}

export interface PageStripProps {
  store: EditorStore;
  layout: Layout;
  page: string;
  open: boolean;
  /** How graph Tiles look, as on the canvas: the samples the Device keeps, and whether its values are stale. */
  live: { series: number; stale: boolean };
}

/**
 * The Pages column between the nav rail and the canvas, like a presentation app's slide list: a thumbnail per Page,
 * then Add Page. Folded away it is inert, so it leaves the tab order and screen readers too.
 */
export function PageStrip({ store, layout, page, open, live }: PageStripProps) {
  const g = useMemo(() => geometry(screenSize(layout.orientation), layout.fullScreen === true), [layout.orientation, layout.fullScreen]);
  const options = useMemo(() => sampleScene(live.series || SERIES, live.stale), [live.series, live.stale]);
  const strip = useRef<HTMLElement>(null);
  /** A Page was removed from its menu, which went with it: focus goes to the Page shown instead of the window. */
  const removed = useRef(false);
  const full = layout.pages.length >= PAGES_MAX;
  const count = layout.pages.length;

  // The column follows the Page on the canvas, however it changed: a click here, Add Page, or ← and → in Preview.
  // Focus on another entry moves along with it, so it never rings a Page that isn't shown.
  useLayoutEffect(() => {
    const scroller = strip.current;
    const entry = scroller?.querySelector<HTMLElement>(`.page-entry[data-page="${CSS.escape(page)}"]`);
    if (!scroller || !entry) return;
    const style = getComputedStyle(scroller);
    const view = scroller.getBoundingClientRect();
    const top = view.top + Number.parseFloat(style.paddingTop);
    const bottom = view.bottom - Number.parseFloat(style.paddingBottom);
    const box = entry.getBoundingClientRect();
    if (box.top < top) scroller.scrollTop -= top - box.top;
    else if (box.bottom > bottom) scroller.scrollTop += box.bottom - bottom;
    const focused = document.activeElement;
    const button = entry.querySelector<HTMLElement>('.page-entry-button');
    const lost = removed.current && (focused === null || focused === document.body);
    removed.current = false;
    const moving = focused !== button && focused instanceof HTMLElement && focused.matches('.page-entry-button') && scroller.contains(focused);
    if (button && (lost || moving)) button.focus({ preventScroll: true });
  }, [page, count]);

  const add = (
    <button
      type="button"
      className="page-add"
      aria-disabled={full}
      onClick={() => !full && store.addPage()}
      data-testid="page-add"
    >
      <Plus size={16} strokeWidth={2} aria-hidden />
      Add Page
    </button>
  );
  return (
    <aside ref={strip} id={PAGES_ID} className="page-strip" aria-label="Pages" inert={!open} data-testid="page-strip" data-open={open}>
      <ol className="page-list">
        {layout.pages.map((p, i) => (
          <PageEntry
            key={p.id}
            store={store}
            page={p}
            onRemove={() => {
              removed.current = true;
              store.removePage(p.id);
            }}
            index={i}
            count={count}
            current={p.id === page}
            g={g}
            options={options}
          />
        ))}
      </ol>
      {full ? <Tip label={`A Layout holds up to ${PAGES_MAX} Pages`}>{add}</Tip> : add}
    </aside>
  );
}
