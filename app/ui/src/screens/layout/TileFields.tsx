import { useEffect, useState, type ReactNode } from 'react';
import { Toggle } from '@base-ui/react/toggle';
import { ToggleGroup } from '@base-ui/react/toggle-group';
import { missingGlyphs } from '../../lib/deck/scene.ts';
import { needsApproval, processOf } from '../../lib/layout/actions.ts';
import { friendlyRefusal } from '../../lib/layout/messages.ts';
import { gifWork } from '../../lib/editor/gifs.ts';
import type { EditorStore } from '../../lib/editor/store.ts';
import type { GridPage, Layout, Tile } from '../../lib/layout/model.ts';
import { ActionForm } from './ActionForm.tsx';
import { ActionLogPanel } from './ActionLog.tsx';
import { AllowedActions, ApprovalNote } from './Approvals.tsx';
import { useDeviceOffersGif, useEditorState, useEffectiveTileEdit, useGifState, useGifTiles } from './editor.ts';
import { described, Field, FieldError, useFieldIds } from './fields.tsx';
import { GifField } from './GifField.tsx';
import { GraphField } from './GraphField.tsx';
import { IconField } from './IconField.tsx';
import { KeyTest } from './KeyTest.tsx';

export interface TileFieldsProps {
  store: EditorStore;
  page: GridPage;
  tile: Tile;
}

const LABEL_MAX = 16;

function LabelField({ store, tile, device }: { store: EditorStore; tile: Tile; device: string }) {
  const ids = useFieldIds();
  const edit = useEffectiveTileEdit(store, device, tile.id);
  if (!edit) return null;
  const { value, error } = edit.label;
  const missing = missingGlyphs(value);
  const hint = `${[...value].length} of ${LABEL_MAX} characters. Long labels shrink on ${device}.`;
  return (
    <Field ids={ids} label="Label" hint={hint} error={error}>
      <input
        id={ids.control}
        className="input"
        value={value}
        maxLength={32}
        spellCheck={false}
        autoComplete="off"
        onChange={(event) => void store.editLabel(tile.id, event.target.value)}
        data-testid="tile-label"
        {...described(ids, hint, error)}
      />
      {missing.length > 0 && (
        <p className="field-hint is-warning">
          {device} can't show {missing.join(' ')} and draws an empty box instead.
        </p>
      )}
    </Field>
  );
}

const SIZES = [
  { value: '1x1', cols: 1, rows: 1 },
  { value: '2x1', cols: 2, rows: 1 },
  { value: '2x2', cols: 2, rows: 2 },
] as const;

function SizeField({ store, tile, layout, device }: { store: EditorStore; tile: Tile; layout: Layout; device: string }) {
  const ids = useFieldIds();
  const [reason, setReason] = useState<string | null>(null);
  useEffect(() => setReason(null), [tile.id, tile.rowSpan, tile.colSpan]);
  const current = SIZES.find((size) => size.cols === tile.colSpan && size.rows === tile.rowSpan)?.value;
  const error = reason ? friendlyRefusal(reason, layout, device) : null;
  return (
    <div className="field">
      <span className="field-label" id={ids.control}>
        Size
      </span>
      <ToggleGroup
        value={current ? [current] : []}
        onValueChange={(value) => {
          const size = SIZES.find((s) => s.value === value[0]);
          if (size) setReason(store.resizeTile(tile.id, size.rows, size.cols));
        }}
        aria-labelledby={ids.control}
        className="segmented segmented-fill"
        aria-describedby={error ? ids.error : undefined}
      >
        {SIZES.map((size) => (
          <Toggle key={size.value} value={size.value} className="segment tabular" data-testid={`tile-size-${size.value}`}>
            {size.cols} × {size.rows}
          </Toggle>
        ))}
      </ToggleGroup>
      <FieldError id={ids.error} error={error} />
    </div>
  );
}

function CellField({ label, value, onCommit, testId }: { label: string; value: number; onCommit(next: number): string | null; testId: string }) {
  const ids = useFieldIds();
  const [draft, setDraft] = useState(String(value));
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setDraft(String(value));
    setError(null);
  }, [value]);
  return (
    <Field ids={ids} label={label} error={error}>
      <input
        id={ids.control}
        className="input tabular"
        type="number"
        min={1}
        max={8}
        value={draft}
        onChange={(event) => {
          setDraft(event.target.value);
          const next = event.target.valueAsNumber;
          if (Number.isInteger(next)) setError(onCommit(next));
        }}
        onBlur={() => {
          setDraft(String(value));
          setError(null);
        }}
        data-testid={testId}
        {...described(ids, null, error)}
      />
    </Field>
  );
}

function PositionFields({ store, tile, layout, device }: { store: EditorStore; tile: Tile; layout: Layout; device: string }) {
  const say = (reason: string | null) => (reason ? friendlyRefusal(reason, layout, device) : null);
  return (
    <div className="inspector-more-body">
      <div className="cell-fields">
        <CellField label="Row" value={tile.row + 1} onCommit={(n) => say(store.moveTile(tile.id, n - 1, tile.col))} testId="tile-row" />
        <CellField label="Column" value={tile.col + 1} onCommit={(n) => say(store.moveTile(tile.id, tile.row, n - 1))} testId="tile-col" />
        <CellField label="Height in rows" value={tile.rowSpan} onCommit={(n) => say(store.resizeTile(tile.id, n, tile.colSpan))} testId="tile-row-span" />
        <CellField label="Width in columns" value={tile.colSpan} onCommit={(n) => say(store.resizeTile(tile.id, tile.rowSpan, n))} testId="tile-col-span" />
      </div>
      <p className="field-hint">You can also drag the Tile or its corner dot. Arrow keys move the selected Tile, and Shift with an arrow key resizes it.</p>
    </div>
  );
}

function More({ title, testId, children }: { title: string; testId: string; children: () => ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <details className="inspector-more" open={open} onToggle={(event) => setOpen(event.currentTarget.open)} data-testid={testId}>
      <summary>{title}</summary>
      {open && children()}
    </details>
  );
}

const KEYBOARD: ReadonlySet<string> = new Set(['key', 'mod', 'consumer']);

export function TileFields({ store, page, tile }: TileFieldsProps) {
  const device = useEditorState(store, (s) => s.device);
  const layout = useEditorState(store, (s) => s.drafts[s.orientation]?.layout);
  const edit = useEffectiveTileEdit(store, device?.name ?? 'the Device', tile.id);
  const orientation = useEditorState(store, (s) => s.orientation);
  const gifs = useGifTiles();
  const offersGif = useDeviceOffersGif(store);
  const working = useGifState(gifs, (s) => gifWork(s, { device, orientation }, tile.id) !== undefined);
  if (!layout || !device || !edit) return null;
  const action = edit.action.value;
  const name = device.name;
  const live = tile.render === 'graph' || tile.render === 'bar' || tile.render === 'now-playing';
  const gif = tile.render === 'gif' || working;
  const showsGif = tile.render !== 'now-playing' && (offersGif || gif) ? { on: gif, choose: (file: File) => gifs.choose(tile.id, file), stop: () => gifs.cancel(tile.id) } : undefined;
  return (
    <div className="tile-fields" data-testid="tile-fields" data-id={tile.id}>
      <LabelField key={`label-${tile.id}`} store={store} tile={tile} device={name} />
      <GraphField store={store} tile={tile} device={name} gif={showsGif} />
      {gif && <GifField store={store} gifs={gifs} tile={tile} device={name} />}
      {!live && !gif && <IconField store={store} tile={tile} kind={action.type} />}
      <SizeField store={store} tile={tile} layout={layout} device={name} />
      {!live && <ActionForm deviceId={device.id} key={`action-${tile.id}`} store={store} page={page} tile={tile} layout={layout} device={name} />}
      {needsApproval(action) && <ApprovalNote action={action} />}
      {processOf(action) && <AllowedActions action={action} />}
      <More key={`position-${tile.id}`} title="Position and size" testId="tile-position">
        {() => <PositionFields store={store} tile={tile} layout={layout} device={name} />}
      </More>
      {KEYBOARD.has(action.type) && (
        <More title="Key test" testId="key-test-panel">
          {() => <KeyTest deviceId={device.id} device={name} />}
        </More>
      )}
      {processOf(action) && (
        <More title="Action log" testId="action-log-panel">
          {() => <ActionLogPanel deviceId={device.id} device={name} layout={layout} />}
        </More>
      )}
    </div>
  );
}
