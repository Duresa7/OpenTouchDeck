// PROTOTYPE — throwaway, answers: how should the Layout editor lay out so the nav never shifts?
// Three structurally different editors. Each owns its layout; they share only state and small parts.
import { useLayoutEffect, useRef, useState } from 'react';
import clsx from 'clsx';
import { Check, ChevronDown, ChevronUp, Eye, Grid3x3, PanelLeftClose, PanelLeftOpen, Plus, SlidersHorizontal, Trash2, X } from 'lucide-react';
import {
  ActionSelect,
  DeviceName,
  DeviceScreen,
  GridPicker,
  IconChoice,
  NavRail,
  OrientationToggle,
  OtherScreen,
  PageList,
  PageThumb,
  SendButton,
  type Deck,
} from './layout-editor.prototype-parts.tsx';

function PreviewToggle({ deck }: { deck: Deck }) {
  return (
    <button
      type="button"
      className="button button-icon"
      data-pressed={deck.preview ? '' : undefined}
      aria-pressed={deck.preview}
      aria-label="Preview"
      title="Preview (P)"
      onClick={() => deck.setPreview(!deck.preview)}
    >
      <Eye size={18} strokeWidth={1.9} aria-hidden />
    </button>
  );
}

function GridButton({ deck, up }: { deck: Deck; up?: boolean }) {
  const [open, setOpen] = useState(false);
  return (
    <span className="proto-pop-anchor">
      <button type="button" className="button proto-grid-button" aria-expanded={open} onClick={() => setOpen(!open)} title="Grid">
        <Grid3x3 size={16} strokeWidth={1.9} aria-hidden />
        <span className="tabular">
          {deck.page.rows} × {deck.page.cols}
        </span>
      </button>
      {open && (
        <span className={clsx('proto-pop', up && 'is-up')}>
          <GridPicker
            rows={deck.page.rows}
            cols={deck.page.cols}
            onPick={(r, c) => {
              deck.setGrid(r, c);
              setOpen(false);
            }}
          />
        </span>
      )}
    </span>
  );
}

function TileCardFields({ deck }: { deck: Deck }) {
  const t = deck.tile!;
  return (
    <>
      <label className="field">
        <span className="field-label">Label</span>
        <span className="proto-row">
          <IconChoice value={t.icon} onPick={(icon) => deck.updateTile({ icon })} />
          <input className="input" value={t.label} onChange={(event) => deck.updateTile({ label: event.target.value })} />
        </span>
      </label>
      <label className="field">
        <span className="field-label">Action</span>
        <ActionSelect value={t.action} onChange={(action) => deck.updateTile({ action })} />
      </label>
    </>
  );
}

/* ======================================================================== A */

/**
 * A — Rail + Page list + overlay Details. The rail floats at the far left, the Pages are an MX-style list beside it,
 * and the Details card floats over the right of the canvas. Opening it moves nothing.
 */
export function VariantA({ deck }: { deck: Deck }) {
  const [pageCard, setPageCard] = useState(false);
  const mode = deck.tile ? 'tile' : pageCard ? 'page' : null;
  return (
    <div className="proto-a">
      <NavRail deck={deck} className="proto-a-rail" />
      {deck.screen !== 'layout' ? (
        <main className="proto-a-other">
          <OtherScreen screen={deck.screen} />
        </main>
      ) : (
        <>
          <aside className="proto-a-side">
            <PageList deck={deck} />
          </aside>
          <main className="proto-a-canvas">
            <header className="proto-head">
              <DeviceName deck={deck} />
              <div className="proto-head-tools">
                <OrientationToggle deck={deck} />
                <PreviewToggle deck={deck} />
                <button
                  type="button"
                  className="button button-icon"
                  data-pressed={mode === 'page' ? '' : undefined}
                  aria-pressed={mode === 'page'}
                  aria-label="Page settings"
                  title="Page settings"
                  onClick={() => {
                    deck.selectTile(null);
                    setPageCard(mode !== 'page');
                  }}
                >
                  <SlidersHorizontal size={18} strokeWidth={1.9} aria-hidden />
                </button>
                <button type="button" className="button" onClick={() => deck.addTile()}>
                  <Plus size={16} strokeWidth={2} aria-hidden />
                  Add Tile
                </button>
                <SendButton deck={deck} />
              </div>
            </header>
            <div className="proto-stage" onClick={() => deck.selectTile(null)}>
              <DeviceScreen deck={deck} onTile={(t) => deck.selectTile(t.id)} onEmpty={(cell) => deck.addTile(cell)} />
            </div>
            {mode && (
              <section key={mode} className="proto-card proto-a-card" aria-label={mode === 'tile' ? 'Tile details' : 'Page details'}>
                <div className="proto-card-head">
                  <span className="proto-card-title">{mode === 'tile' ? deck.tile!.label || 'Tile' : deck.page.title}</span>
                  <button
                    type="button"
                    className="button button-icon"
                    aria-label="Close"
                    onClick={() => {
                      deck.selectTile(null);
                      setPageCard(false);
                    }}
                  >
                    <X size={16} strokeWidth={1.9} aria-hidden />
                  </button>
                </div>
                {mode === 'tile' ? (
                  <>
                    <TileCardFields deck={deck} />
                    {deck.tile!.value && <p className="field-hint">Shows {deck.tile!.value} now.</p>}
                    <div className="proto-card-foot">
                      <button type="button" className="button button-quiet-danger" onClick={deck.deleteTile}>
                        <Trash2 size={16} strokeWidth={1.9} aria-hidden />
                        Remove Tile
                      </button>
                    </div>
                  </>
                ) : (
                  <>
                    <label className="field">
                      <span className="field-label">Page name</span>
                      <input className="input" value={deck.page.title} maxLength={24} onChange={(event) => deck.renamePage(event.target.value)} />
                    </label>
                    <div className="field">
                      <span className="field-label">Grid</span>
                      <GridPicker rows={deck.page.rows} cols={deck.page.cols} onPick={deck.setGrid} />
                    </div>
                  </>
                )}
              </section>
            )}
          </main>
        </>
      )}
    </div>
  );
}

/* ======================================================================== B */

const CARD_W = 272;

/**
 * B — Rail + Page list + toolbar. The editing tools float in a toolbar above the Device, and picking a Tile opens a
 * small card next to that Tile. There is no side panel at all.
 */
export function VariantB({ deck }: { deck: Deck }) {
  const stage = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number; side: 'left' | 'right' } | null>(null);
  const [more, setMore] = useState(false);
  const tileId = deck.tile?.id ?? null;

  useLayoutEffect(() => {
    const measure = () => {
      const box = stage.current;
      const el = tileId && box?.querySelector(`[data-tile-id="${tileId}"]`);
      if (!box || !el) {
        setPos(null);
        return;
      }
      const s = box.getBoundingClientRect();
      const r = el.getBoundingClientRect();
      const right = r.right - s.left + 12;
      const side = right + CARD_W < s.width - 8 ? 'right' : 'left';
      const left = side === 'right' ? right : r.left - s.left - 12 - CARD_W;
      const top = Math.max(8, Math.min(r.top - s.top - 6, s.height - 300));
      setPos({ left, top, side });
    };
    measure();
    window.addEventListener('resize', measure);
    return () => window.removeEventListener('resize', measure);
  }, [tileId, deck.orientation, deck.page.id, deck.page.rows, deck.page.cols]);

  useLayoutEffect(() => setMore(false), [tileId]);

  return (
    <div className="proto-b">
      <NavRail deck={deck} className="proto-b-rail" />
      {deck.screen !== 'layout' ? (
        <main className="proto-b-other">
          <OtherScreen screen={deck.screen} />
        </main>
      ) : (
        <>
          <aside className="proto-b-side">
            <div className="proto-b-device">
              <DeviceName deck={deck} />
            </div>
            <PageList deck={deck} />
          </aside>
          <main className="proto-b-canvas" ref={stage} onClick={() => deck.selectTile(null)}>
            <div className="proto-toolbar proto-b-toolbar" onClick={(event) => event.stopPropagation()}>
              <input
                className="input proto-toolbar-name"
                value={deck.page.title}
                maxLength={24}
                aria-label="Page name"
                onChange={(event) => deck.renamePage(event.target.value)}
              />
              <GridButton deck={deck} />
              <span className="proto-toolbar-sep" />
              <OrientationToggle deck={deck} />
              <PreviewToggle deck={deck} />
              <span className="proto-toolbar-sep" />
              <button type="button" className="button" onClick={() => deck.addTile()}>
                <Plus size={16} strokeWidth={2} aria-hidden />
                Add Tile
              </button>
              <SendButton deck={deck} />
            </div>
            <div className="proto-b-stage">
              <DeviceScreen deck={deck} onTile={(t) => deck.selectTile(t.id)} onEmpty={(cell) => deck.addTile(cell)} />
            </div>
            {deck.tile && pos && (
              <section
                key={deck.tile.id}
                className={clsx('proto-card proto-b-card', `is-${pos.side}`)}
                style={{ left: pos.left, top: pos.top, width: CARD_W }}
                onClick={(event) => event.stopPropagation()}
                aria-label="Tile details"
              >
                <TileCardFields deck={deck} />
                {more && (
                  <>
                    {deck.tile.value && <p className="field-hint">Shows {deck.tile.value} now.</p>}
                    <button type="button" className="button button-quiet-danger button-small" onClick={deck.deleteTile}>
                      <Trash2 size={14} strokeWidth={1.9} aria-hidden />
                      Remove Tile
                    </button>
                  </>
                )}
                <div className="proto-card-foot is-split">
                  <button type="button" className="button button-small" onClick={() => setMore(!more)} aria-expanded={more}>
                    {more ? <ChevronUp size={14} strokeWidth={2} aria-hidden /> : <ChevronDown size={14} strokeWidth={2} aria-hidden />}
                    {more ? 'Less' : 'More'}
                  </button>
                  <button type="button" className="button button-small" onClick={() => deck.selectTile(null)}>
                    Done
                  </button>
                </div>
              </section>
            )}
          </main>
        </>
      )}
    </div>
  );
}

/* ======================================================================== C */

/**
 * C — Rail + bottom contextual toolbar. The rail is a full-height strip with the Pages docked against it as a
 * filmstrip; every edit happens in one toolbar along the bottom of the canvas that changes with the selection.
 */
export function VariantC({ deck }: { deck: Deck }) {
  const [strip, setStrip] = useState(true);
  const mode = deck.tile ? 'tile' : 'page';
  return (
    <div className={clsx('proto-c', !strip && 'is-strip-closed')}>
      <NavRail deck={deck} className="proto-c-rail" />
      {deck.screen !== 'layout' ? (
        <main className="proto-c-other">
          <OtherScreen screen={deck.screen} />
        </main>
      ) : (
        <>
          <aside className="proto-c-strip" aria-label="Pages" inert={!strip}>
            <ul className="proto-c-pages">
              {deck.pages.map((p, i) => (
                <li key={p.id}>
                  <button
                    type="button"
                    className={clsx('proto-c-page', p.id === deck.page.id && 'is-current')}
                    aria-current={p.id === deck.page.id ? 'true' : undefined}
                    onClick={() => deck.selectPage(p.id)}
                  >
                    <span className="proto-c-page-num tabular">{i + 1}</span>
                    <PageThumb page={p} />
                    <span className="proto-c-page-name">{p.title}</span>
                  </button>
                </li>
              ))}
            </ul>
            <button type="button" className="proto-c-add" onClick={deck.addPage}>
              <Plus size={16} strokeWidth={2} aria-hidden />
              Add Page
            </button>
          </aside>
          <main className="proto-c-canvas" onClick={() => deck.selectTile(null)}>
            <header className="proto-head" onClick={(event) => event.stopPropagation()}>
              <div className="proto-row">
                <button type="button" className="button button-icon" aria-label={strip ? 'Hide Pages' : 'Show Pages'} title={strip ? 'Hide Pages' : 'Show Pages'} onClick={() => setStrip(!strip)}>
                  {strip ? <PanelLeftClose size={18} strokeWidth={1.9} aria-hidden /> : <PanelLeftOpen size={18} strokeWidth={1.9} aria-hidden />}
                </button>
                <DeviceName deck={deck} />
              </div>
              <div className="proto-head-tools">
                <SendButton deck={deck} />
              </div>
            </header>
            <div className="proto-stage">
              <DeviceScreen deck={deck} onTile={(t) => deck.selectTile(t.id)} onEmpty={(cell) => deck.addTile(cell)} />
            </div>
            <div className="proto-toolbar proto-c-toolbar" onClick={(event) => event.stopPropagation()} data-mode={mode}>
              <div key={mode} className="proto-c-swap">
                {mode === 'page' ? (
                  <>
                    <span className="proto-toolbar-tag">Page</span>
                    <input
                      className="input proto-toolbar-name"
                      value={deck.page.title}
                      maxLength={24}
                      aria-label="Page name"
                      onChange={(event) => deck.renamePage(event.target.value)}
                    />
                    <GridButton deck={deck} up />
                    <span className="proto-toolbar-sep" />
                    <OrientationToggle deck={deck} />
                    <PreviewToggle deck={deck} />
                    <span className="proto-toolbar-sep" />
                    <button type="button" className="button" onClick={() => deck.addTile()}>
                      <Plus size={16} strokeWidth={2} aria-hidden />
                      Add Tile
                    </button>
                  </>
                ) : (
                  <>
                    <span className="proto-toolbar-tag">Tile</span>
                    <IconChoice value={deck.tile!.icon} onPick={(icon) => deck.updateTile({ icon })} up />
                    <input
                      className="input proto-toolbar-name"
                      value={deck.tile!.label}
                      aria-label="Label"
                      onChange={(event) => deck.updateTile({ label: event.target.value })}
                    />
                    <ActionSelect value={deck.tile!.action} onChange={(action) => deck.updateTile({ action })} className="proto-toolbar-action" />
                    <span className="proto-toolbar-sep" />
                    <button type="button" className="button button-icon" aria-label="Remove Tile" title="Remove Tile" onClick={deck.deleteTile}>
                      <Trash2 size={17} strokeWidth={1.9} aria-hidden />
                    </button>
                    <button type="button" className="button button-small" onClick={() => deck.selectTile(null)}>
                      <Check size={14} strokeWidth={2.2} aria-hidden />
                      Done
                    </button>
                  </>
                )}
              </div>
            </div>
          </main>
        </>
      )}
    </div>
  );
}
