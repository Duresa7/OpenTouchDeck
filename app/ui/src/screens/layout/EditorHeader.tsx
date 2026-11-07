import clsx from 'clsx';
import { Menu } from '@base-ui/react/menu';
import { Toggle } from '@base-ui/react/toggle';
import { ToggleGroup } from '@base-ui/react/toggle-group';
import { Check, Eye, MoreHorizontal, Plus, RectangleHorizontal, RectangleVertical, Upload } from 'lucide-react';
import type { DeviceSummary } from '../../lib/companion/api.ts';
import { saveState, type EditorStore } from '../../lib/editor/store.ts';
import type { Orientation } from '../../lib/layout/model.ts';
import { button } from '../../shell/button.ts';
import { deviceStatus } from '../home-model.ts';
import { DevicePanel } from './DevicePanel.tsx';
import { useEditorState } from './editor.ts';
import { Tip } from './Tip.tsx';

const SAVE_WORDS = { saved: 'Saved on Device', unsent: 'Unsent changes', sending: 'Sending…' } as const;

function SaveState({ store }: { store: EditorStore }) {
  const state = useEditorState(store, saveState);
  if (state === 'none') return <span className="save-state" />;
  return (
    <span className={clsx('save-state', `is-${state}`)} data-testid="save-state" data-state={state}>
      {state === 'saved' ? <Check size={14} strokeWidth={2} aria-hidden /> : <i className="save-dot" aria-hidden />}
      <span className="save-words">{SAVE_WORDS[state]}</span>
    </span>
  );
}

const ORIENTATIONS: { value: Orientation; label: string; icon: typeof RectangleHorizontal }[] = [
  { value: 'landscape', label: 'Landscape', icon: RectangleHorizontal },
  { value: 'portrait', label: 'Portrait', icon: RectangleVertical },
];

function LayoutMenu({ store, device, connected, onImport, onExport }: { store: EditorStore; device: string; connected: boolean; onImport(): void; onExport(): void }) {
  const busy = useEditorState(store, (s) => s.busy);
  const hasDraft = useEditorState(store, (s) => s.drafts[s.orientation] !== null);
  return (
    <Menu.Root>
      <Tip label="More">
        <Menu.Trigger className={button({ kind: 'icon' })} aria-label="Layout options" data-testid="layout-menu">
          <MoreHorizontal size={18} strokeWidth={1.9} aria-hidden />
        </Menu.Trigger>
      </Tip>
      <Menu.Portal>
        <Menu.Positioner className="menu-positioner" side="bottom" align="end" sideOffset={6}>
          <Menu.Popup className="menu" aria-label="Layout options">
            <Menu.Item className="menu-item" disabled={!connected || busy !== null} onClick={() => void store.load()} data-testid="pull">
              <span className="menu-name">{busy === 'load' ? 'Loading…' : `Load from ${device}`}</span>
              <span className="menu-line">{connected ? `Replace this Layout with the one on ${device}` : `${device} isn't connected`}</span>
            </Menu.Item>
            <Menu.Separator className="menu-separator" />
            <Menu.Item className="menu-item" onClick={onImport} data-testid="import">
              <span className="menu-name">Import</span>
              <span className="menu-line">A Layout file you exported</span>
            </Menu.Item>
            <Menu.Item className="menu-item" disabled={!hasDraft} onClick={onExport} data-testid="export">
              <span className="menu-name">Export</span>
              <span className="menu-line">Save a copy to share or keep</span>
            </Menu.Item>
            <Menu.Separator className="menu-separator" />
            <Menu.Item className="menu-item" onClick={() => store.startStarter()} data-testid="menu-starter">
              <span className="menu-name">Starter Layout</span>
              <span className="menu-line">Media controls and shortcuts</span>
            </Menu.Item>
            <Menu.Item className="menu-item" onClick={() => store.startBlank()} data-testid="menu-new">
              <span className="menu-name">Blank Layout</span>
              <span className="menu-line">One empty Page</span>
            </Menu.Item>
            <Menu.Separator className="menu-separator" />
            <Menu.Item className="menu-item" disabled={!connected || busy !== null} onClick={() => void store.pruneAssets()} data-testid="prune">
              <span className="menu-name">Remove unused Assets</span>
              <span className="menu-line">{connected ? `Frees space on ${device}` : `${device} isn't connected`}</span>
            </Menu.Item>
          </Menu.Popup>
        </Menu.Positioner>
      </Menu.Portal>
    </Menu.Root>
  );
}

export interface EditorHeaderProps {
  store: EditorStore;
  device: DeviceSummary;
  onImport(): void;
  onExport(): void;
}

export function EditorHeader({ store, device, onImport, onExport }: EditorHeaderProps) {
  const name = device.name ?? 'Unnamed Device';
  const status = deviceStatus(device);
  const orientation = useEditorState(store, (s) => s.orientation);
  const mode = useEditorState(store, (s) => s.mode);
  const busy = useEditorState(store, (s) => s.busy);
  const adding = useEditorState(store, (s) => s.adding !== null);
  const hasDraft = useEditorState(store, (s) => s.drafts[s.orientation] !== null);
  const canSend = status.live && hasDraft && busy === null;

  return (
    <header className="editor-head">
      <DevicePanel device={device} />
      <span className={clsx('status', status.live && 'is-live')} title={status.words} data-testid="connection">
        <i className="led" aria-hidden />
        <span className="status-words">{status.words}</span>
      </span>
      <SaveState store={store} />
      <ToggleGroup
        value={[orientation]}
        onValueChange={(value) => value[0] && store.setOrientation(value[0])}
        aria-label="Orientation"
        className="segmented segmented-icons"
      >
        {ORIENTATIONS.map((o) => (
          <Tip key={o.value} label={o.label}>
            <Toggle value={o.value} className="segment" aria-label={o.label} data-testid={`orientation-${o.value}`}>
              <o.icon size={16} strokeWidth={1.9} aria-hidden />
            </Toggle>
          </Tip>
        ))}
      </ToggleGroup>
      {hasDraft && (
        <Tip label="Preview (P)">
          <Toggle
            pressed={mode === 'preview'}
            onPressedChange={(pressed) => store.setMode(pressed ? 'preview' : 'edit')}
            className={button({ kind: 'icon' })}
            aria-label="Preview"
            data-testid="preview"
          >
            <Eye size={18} strokeWidth={1.9} aria-hidden />
          </Toggle>
        </Tip>
      )}
      <LayoutMenu store={store} device={name} connected={status.live} onImport={onImport} onExport={onExport} />
      {hasDraft && (
        <>
          <button
            type="button"
            className={clsx(button(), 'add-tile-button')}
            aria-expanded={adding}
            onClick={() => (adding ? store.closeAdd() : store.openAdd())}
            data-testid="add-tile"
          >
            <Plus size={16} strokeWidth={2} aria-hidden />
            <span className="add-tile-words">Add Tile</span>
          </button>
          <Tip label={status.live ? `Send this Layout to ${name}` : `${name} isn't connected`}>
            <button
              type="button"
              className={clsx(button({ kind: 'primary' }), 'push-button')}
              aria-disabled={!canSend}
              onClick={() => canSend && void store.push()}
              data-testid="push"
            >
              <Upload size={16} strokeWidth={2} aria-hidden />
              <span className="push-words">{busy === 'push' ? 'Sending…' : 'Send to Device'}</span>
            </button>
          </Tip>
        </>
      )}
    </header>
  );
}
