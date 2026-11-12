import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react';
import clsx from 'clsx';
import { Switch } from '@base-ui/react/switch';
import { Tabs } from '@base-ui/react/tabs';
import { Film, Plus, X } from 'lucide-react';
import type { IntegrationSummary } from '../../lib/companion/api.ts';
import { useDevices } from '../../lib/companion/devices.ts';
import { useIntegrations } from '../../lib/companion/integrations.ts';
import { missingGlyphs } from '../../lib/deck/scene.ts';
import { placedLook } from '../../lib/editor/graph.ts';
import { ADD_KINDS, type EditorStore, type Target } from '../../lib/editor/store.ts';
import { plural } from '../../lib/layout/messages.ts';
import { PAGES_MAX, type GridPage, type Layout, type Page } from '../../lib/layout/model.ts';
import { offersPresets } from '../../lib/integrations/model.ts';
import { hasSettings } from '../../lib/integrations/switch.ts';
import { button } from '../../shell/button.ts';
import { useUi } from '../../state/ui.ts';
import { IntegrationGlyph } from '../integrations/IntegrationGlyph.tsx';
import { addTab, previewGrid, type AddTab } from './add-tile.ts';
import { PresetCard } from './PresetCard.tsx';
import { usePresetNeeds } from './presets.ts';
import { PendingApprovals } from './Approvals.tsx';
import { ControlReview } from './Controls.tsx';
import { useDeviceOffersGif, useDeviceShows, useEditorState, useEffectiveTileEdit, useGifTiles } from './editor.ts';
import { GifInput } from './GifField.tsx';
import { TileIcon } from './AssetImage.tsx';
import { KINDS } from './glyphs.tsx';
import { TileFields } from './TileFields.tsx';
import { Tip } from './Tip.tsx';

function GlyphWarning({ value, device }: { value: string; device: string }) {
  const missing = missingGlyphs(value);
  if (!missing.length) return null;
  return (
    <p className="field-hint is-warning">
      {device} can't show {missing.join(' ')} and draws an empty box instead.
    </p>
  );
}

function NameField({ label, value, device, onCommit, testId }: { label: string; value: string; device: string; onCommit(next: string): string | null; testId: string }) {
  const id = useId();
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const commit = () => {
    if (draft === value) return;
    if (onCommit(draft) !== null) setDraft(value);
  };
  const keys = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') commit();
    if (event.key === 'Escape' && draft !== value) {
      event.stopPropagation();
      setDraft(value);
    }
  };
  return (
    <div className="field">
      <label className="field-label" htmlFor={id}>
        {label}
      </label>
      <input
        id={id}
        className="input"
        value={draft}
        maxLength={24}
        spellCheck={false}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={commit}
        onKeyDown={keys}
        data-testid={testId}
      />
      <GlyphWarning value={draft} device={device} />
    </div>
  );
}

/** The id of the Details panel, which the header's panel button opens and closes. */
export const DETAILS_ID = 'editor-details';

const GRID_MAX = 8;
const SIZES = Array.from({ length: GRID_MAX }, (_, i) => i + 1);

interface GridSize {
  rows: number;
  cols: number;
}

const gridWords = ({ rows, cols }: GridSize) => `${plural(rows, 'row')} × ${plural(cols, 'column')}`;
const clampSize = (n: number) => Math.min(GRID_MAX, Math.max(1, n));

const MOVES: Partial<Record<string, GridSize>> = {
  ArrowUp: { rows: -1, cols: 0 },
  ArrowDown: { rows: 1, cols: 0 },
  ArrowLeft: { rows: 0, cols: -1 },
  ArrowRight: { rows: 0, cols: 1 },
};

/**
 * Pick the grid by sweeping over an 8 × 8 board of squares, like inserting a table in a word processor. Hovering or
 * moving focus previews the size; a click, Enter or Space sets it. Only one square is in the tab order, and the
 * arrow keys move between them.
 */
function GridField({ store, page }: { store: EditorStore; page: GridPage }) {
  const id = useId();
  const board = useRef<HTMLDivElement>(null);
  const [preview, setPreview] = useState<GridSize | null>(null);
  const [focus, setFocus] = useState<GridSize>({ rows: page.rows, cols: page.cols });
  useEffect(() => setFocus({ rows: page.rows, cols: page.cols }), [page.id, page.rows, page.cols]);
  const shown = preview ?? { rows: page.rows, cols: page.cols };

  const keys = (event: KeyboardEvent<HTMLDivElement>) => {
    const move = MOVES[event.key];
    const next = move
      ? { rows: clampSize(focus.rows + move.rows), cols: clampSize(focus.cols + move.cols) }
      : event.key === 'Home'
        ? { rows: 1, cols: 1 }
        : event.key === 'End'
          ? { rows: GRID_MAX, cols: GRID_MAX }
          : null;
    if (!next) return;
    event.preventDefault();
    setFocus(next);
    setPreview(next);
    board.current?.querySelector<HTMLButtonElement>(`[data-rows="${next.rows}"][data-cols="${next.cols}"]`)?.focus();
  };

  return (
    <div className="field">
      <div className="grid-picker-head">
        <span className="field-label" id={id}>
          Grid
        </span>
        <span className={clsx('grid-picker-size tabular', preview && 'is-preview')} aria-hidden data-testid="grid-size">
          {gridWords(shown)}
        </span>
      </div>
      <div
        ref={board}
        className="grid-picker"
        role="group"
        aria-labelledby={id}
        onKeyDown={keys}
        onPointerLeave={() => setPreview(null)}
        onBlur={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget)) setPreview(null);
        }}
        data-testid="grid-picker"
      >
        {SIZES.map((rows) =>
          SIZES.map((cols) => {
            const size = { rows, cols };
            return (
              <button
                key={`${rows}x${cols}`}
                type="button"
                className="grid-picker-cell"
                tabIndex={rows === focus.rows && cols === focus.cols ? 0 : -1}
                aria-label={gridWords(size)}
                aria-pressed={rows === page.rows && cols === page.cols}
                data-lit={rows <= shown.rows && cols <= shown.cols ? (preview ? 'preview' : 'set') : undefined}
                data-rows={rows}
                data-cols={cols}
                onPointerEnter={() => setPreview(size)}
                onFocus={() => {
                  setFocus(size);
                  setPreview(size);
                }}
                onClick={() => {
                  setPreview(null);
                  if (rows !== page.rows || cols !== page.cols) store.setGrid(rows, cols);
                }}
                data-testid="grid-cell"
              />
            );
          }),
        )}
      </div>
    </div>
  );
}

function Details({ store, layout, page, device }: { store: EditorStore; layout: Layout; page: Page; device: string }) {
  const index = layout.pages.findIndex((p) => p.id === page.id);
  const fullId = useId();
  return (
    <>
      <div className="inspector-details" data-testid="page-details">
        <ControlReview store={store} layout={layout} />
        <section className="inspector-section" aria-labelledby={`${fullId}-page`}>
          <div className="section-head">
            <h2 className="section-title" id={`${fullId}-page`}>
              {page.kind === 'dashboard' ? 'Usage page' : 'Page'}
            </h2>
            <span className="section-sub tabular">
              Page {index + 1} of {layout.pages.length}
            </span>
          </div>
          <NameField
            key={page.id}
            label="Name"
            value={page.title}
            device={device}
            onCommit={(title) => store.renamePage(page.id, title)}
            testId="page-name"
          />
          {page.kind === 'grid' ? (
            <GridField store={store} page={page} />
          ) : (
            <p className="field-hint">Lists each CLIProxyAPI account and how much of its limit is used.</p>
          )}
        </section>
        <section className="inspector-section" aria-labelledby={`${fullId}-layout`}>
          <div className="section-head">
            <h2 className="section-title" id={`${fullId}-layout`}>
              Layout
            </h2>
          </div>
          <NameField label="Name" value={layout.name} device={device} onCommit={(name) => store.renameLayout(name)} testId="layout-name" />
          <div className="field">
            <div className="switch-row">
              <span className="field-label" id={`${fullId}-full`}>
                Full screen
              </span>
              <Switch.Root
                className="switch"
                checked={layout.fullScreen === true}
                onCheckedChange={(on) => store.setFullScreen(on)}
                aria-labelledby={`${fullId}-full`}
                data-testid="full-screen"
              >
                <Switch.Thumb className="switch-thumb" />
              </Switch.Root>
            </div>
            <p className="field-hint">Hide the Page name and dots so Tiles fill the screen. Swiping between Pages still works.</p>
          </div>
        </section>
      </div>
    </>
  );
}

function TileInspector({ store, page, tileId }: { store: EditorStore; page: GridPage; tileId: string }) {
  const tile = page.tiles.find((t) => t.id === tileId);
  const device = useEditorState(store, (s) => s.device?.name ?? 'the Device');
  const edit = useEffectiveTileEdit(store, device, tileId);
  if (!tile || !edit) return null;
  const kind = edit.kind;
  return (
    <>
      <div className="inspector-head" data-testid="tile-head">
        <span className="tile-thumb" aria-hidden>
          <TileIcon tile={tile} kind={kind} size={22} />
        </span>
        <div className="inspector-head-text">
          <h2 className="inspector-title" data-testid="tile-title">
            {tile.label}
          </h2>
          <p className="inspector-sub">
            {tile.render === 'gif' && kind === 'none' ? 'GIF' : KINDS[kind].noun} on the {page.title} Page
          </p>
        </div>
        <Tip label="Close (Esc)">
          <button type="button" className={clsx(button({ kind: 'icon' }), 'inspector-close')} aria-label="Close Tile details" onClick={() => store.select(null)} data-testid="tile-close">
            <X size={18} strokeWidth={1.9} aria-hidden />
          </button>
        </Tip>
      </div>
      <TileFields store={store} page={page} tile={tile} />
      <div className="inspector-foot">
        <button type="button" className={button()} onClick={() => store.duplicate(tile.id)} data-testid="tile-duplicate">
          Duplicate
        </button>
        <button type="button" className={button({ kind: 'danger' })} onClick={() => store.clearTile(tile.id)} data-testid="tile-clear">
          Clear Tile
        </button>
      </div>
    </>
  );
}

function where(target: Target, page: Page, pages: number) {
  if (target === 'new-page' && pages >= PAGES_MAX) return `The ${page.title} Page is full. A Layout holds up to ${PAGES_MAX} Pages.`;
  if (target === 'new-page') return `The ${page.title} Page is full, so it starts a new Page`;
  return `Row ${target.row + 1}, column ${target.col + 1} on the ${page.title} Page`;
}

function PresetGroup({ store, integration, rows, cols }: { store: EditorStore; integration: IntegrationSummary; rows: number; cols: number }) {
  const openIntegration = useUi((s) => s.openIntegration);
  const graphs = useDeviceShows(store, 'graph');
  const needs = usePresetNeeds();
  const orientation = useEditorState(store, (s) => s.orientation);
  const fullScreen = useEditorState(store, (s) => s.drafts[s.orientation]?.layout.fullScreen === true);
  const deviceId = useEditorState(store, (s) => s.device?.id);
  const series = useDevices((s) => s.devices.find((d) => d.id === deviceId)?.capabilities?.series || 32);
  const options = { rows, cols, orientation, fullScreen, graphs, series };
  const titleId = useId();
  return (
    <section className="preset-group" aria-labelledby={titleId} data-testid="preset-group" data-id={integration.id}>
      <div className="preset-group-head">
        <IntegrationGlyph icon={integration.icon} size={16} />
        <h3 className="preset-group-name" id={titleId}>
          {integration.name}
        </h3>
        {hasSettings(integration) && (
          <button type="button" className="link-button preset-group-settings" onClick={() => openIntegration(integration.id)} aria-label={`${integration.name} settings`} data-testid="preset-group-settings">
            Settings
          </button>
        )}
      </div>
      <div className="preset-grid">
        {integration.presets.map((preset) => (
          <PresetCard key={preset.id} preset={preset} integration={integration} options={options} needs={needs(preset)} onAdd={(chosen) => store.placePreset(chosen, placedLook(chosen, integration, graphs))} />
        ))}
      </div>
    </section>
  );
}

/** Preset tiles from every Integration that is on and set up, drawn as they will look. */
function IntegrationTiles({ store, page, target }: { store: EditorStore; page: Page; target: Target }) {
  const integrations = useIntegrations((s) => s.integrations);
  const go = useUi((s) => s.go);
  const offered = integrations.filter(offersPresets);
  const { rows, cols } = previewGrid(page, target === 'new-page');
  return (
    <div className="add-integrations" data-testid="add-presets">
      {offered.length ? (
        offered.map((integration) => <PresetGroup key={integration.id} store={store} integration={integration} rows={rows} cols={cols} />)
      ) : (
        <div className="add-empty" data-testid="add-presets-empty">
          <p className="add-empty-text">Turn on an Integration to add its Tiles.</p>
          <div>
            <button type="button" className={button({ size: 'small' })} onClick={() => go('integrations')} data-testid="add-presets-setup">
              Open Integrations
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

function AddGif({ store }: { store: EditorStore }) {
  const offered = useDeviceOffersGif(store);
  const gifs = useGifTiles();
  const file = useRef<HTMLInputElement>(null);
  if (!offered) return null;
  return (
    <section className="inspector-section" aria-label="Animation" data-testid="add-gif-section">
      <h3 className="add-heading">Animation</h3>
      <div className="add-actions">
        <button type="button" className="add-action" onClick={() => file.current?.click()} data-testid="add-gif">
          <span className="add-action-glyph" aria-hidden>
            <Film size={16} strokeWidth={1.9} />
          </span>
          <span className="add-action-text">
            <span className="add-action-name">GIF</span>
            <span className="add-action-line">Plays an animated GIF</span>
          </span>
        </button>
      </div>
      <GifInput inputRef={file} onFile={(chosen) => gifs.add(chosen)} testId="add-gif-file" />
    </section>
  );
}

function AddActions({ store }: { store: EditorStore }) {
  return (
    <>
      <section className="inspector-section" aria-label="Action">
        <h3 className="add-heading">Action</h3>
        <div className="add-actions">
          {ADD_KINDS.map((kind) => {
            const Icon = KINDS[kind].icon;
            return (
              <button key={kind} type="button" className="add-action" onClick={() => store.place(kind)} data-testid="add-kind" data-kind={kind}>
                <span className="add-action-glyph" aria-hidden>
                  <Icon size={16} strokeWidth={1.9} />
                </span>
                <span className="add-action-text">
                  <span className="add-action-name">{KINDS[kind].name}</span>
                  <span className="add-action-line">{KINDS[kind].line}</span>
                </span>
              </button>
            );
          })}
        </div>
      </section>
      <AddGif store={store} />
    </>
  );
}

function AddTile({ store, page, pages, target }: { store: EditorStore; page: Page; pages: number; target: Target }) {
  const integrations = useIntegrations((s) => s.integrations);
  const chosen = useUi((s) => s.addTileTab);
  const choose = useUi((s) => s.setAddTileTab);
  const tab = addTab(integrations, chosen);
  const tabs = useRef<HTMLDivElement>(null);
  // Focus the open tab once the tabs have registered, so the arrow keys move from it.
  useEffect(() => {
    let frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(() => tabs.current?.querySelector<HTMLElement>('[aria-selected="true"]')?.focus({ preventScroll: true }));
    });
    return () => cancelAnimationFrame(frame);
  }, []);
  return (
    <>
      <div className="inspector-head">
        <span className="tile-thumb tile-thumb-add" aria-hidden>
          <Plus size={22} strokeWidth={2} />
        </span>
        <div className="inspector-head-text">
          <h2 className="inspector-title">Add Tile</h2>
          <p className="inspector-sub" data-testid="add-where">
            {where(target, page, pages)}
          </p>
        </div>
      </div>
      <Tabs.Root className="add-tabs" value={tab} onValueChange={(next: AddTab) => choose(next)}>
        <Tabs.List ref={tabs} className="segmented segmented-fill add-tab-list" aria-label="Add from" activateOnFocus>
          <Tabs.Tab className="segment" value="actions" data-testid="add-tab-actions">
            Actions
          </Tabs.Tab>
          <Tabs.Tab className="segment" value="integrations" data-testid="add-tab-integrations">
            Integrations
          </Tabs.Tab>
        </Tabs.List>
        <Tabs.Panel className="add-panel" value="actions" data-testid="add-panel-actions">
          <AddActions store={store} />
        </Tabs.Panel>
        <Tabs.Panel className="add-panel" value="integrations" data-testid="add-panel-integrations">
          <IntegrationTiles store={store} page={page} target={target} />
        </Tabs.Panel>
      </Tabs.Root>
      <div className="inspector-foot">
        <button type="button" className={button()} onClick={() => store.closeAdd()} data-testid="add-cancel">
          Cancel
        </button>
      </div>
    </>
  );
}

export function Inspector({ store, layout, device }: { store: EditorStore; layout: Layout; device: string }) {
  const pageId = useEditorState(store, (s) => s.page);
  const selected = useEditorState(store, (s) => s.selected);
  const adding = useEditorState(store, (s) => s.adding);
  const mode = useEditorState(store, (s) => s.mode);
  const deviceId = useEditorState(store, (s) => s.device?.id);
  const page = layout.pages.find((p) => p.id === pageId) ?? layout.pages[0];
  const tileOpen = mode === 'edit' && selected !== null && page.kind === 'grid' && page.tiles.some((t) => t.id === selected);
  return (
    <aside id={DETAILS_ID} className={clsx('inspector', adding !== null && 'is-adding')} aria-label="Details" data-testid="inspector">
      {deviceId && <PendingApprovals deviceId={deviceId} device={device} layout={layout} />}
      {adding !== null ? (
        <AddTile store={store} page={page} pages={layout.pages.length} target={adding} />
      ) : tileOpen && page.kind === 'grid' ? (
        <TileInspector store={store} page={page} tileId={selected!} />
      ) : (
        <Details store={store} layout={layout} page={page} device={device} />
      )}
    </aside>
  );
}
