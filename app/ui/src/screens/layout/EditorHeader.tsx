import clsx from 'clsx';
import { Menu } from '@base-ui/react/menu';
import { Toggle } from '@base-ui/react/toggle';
import { ToggleGroup } from '@base-ui/react/toggle-group';
import { Eye, MoreHorizontal, PanelRightClose, PanelRightOpen, Plus, RectangleHorizontal, RectangleVertical } from 'lucide-react';
import type { DeviceSummary } from '../../lib/companion/api.ts';
import { saveState, type EditorStore } from '../../lib/editor/store.ts';
import type { Orientation } from '../../lib/layout/model.ts';
import { button } from '../../shell/button.ts';
import { openDetails, useUi } from '../../state/ui.ts';
import { deviceStatus } from '../home-model.ts';
import { DeviceName } from './DeviceName.tsx';
import { useEditorState } from './editor.ts';
import { DETAILS_ID } from './Inspector.tsx';
import { SendButton } from './SendButton.tsx';
import { Tip } from './Tip.tsx';

/** Hiding the panel also ends Add Tile, whose picker lives there, so Add Tile never stays open out of sight. */
function DetailsToggle({ store }: { store: EditorStore }) {
  const open = useUi((s) => s.detailsOpen);
  const setOpen = useUi((s) => s.setDetailsOpen);
  const label = open ? 'Hide details' : 'Show details';
  const Icon = open ? PanelRightClose : PanelRightOpen;
  return (
    <Tip label={label}>
      <button
        type="button"
        className={button({ kind: 'icon' })}
        aria-label={label}
        aria-expanded={open}
        aria-controls={DETAILS_ID}
        onClick={() => {
          if (open && store.getState().adding !== null) store.closeAdd();
          setOpen(!open);
        }}
        data-testid="details-toggle"
      >
        <Icon size={18} strokeWidth={1.9} aria-hidden />
      </button>
    </Tip>
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
            <Menu.Item className="menu-item" disabled={!connected || !hasDraft || busy !== null} onClick={() => void store.push()} data-testid="menu-push">
              <span className="menu-name">{busy === 'push' ? 'Sending…' : `Send to ${device}`}</span>
              <span className="menu-line">{connected ? 'Send this Layout again, even without changes' : `${device} isn't connected`}</span>
            </Menu.Item>
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
  const adding = useEditorState(store, (s) => s.adding !== null);
  const hasDraft = useEditorState(store, (s) => s.drafts[s.orientation] !== null);
  const saved = useEditorState(store, saveState);

  return (
    <header className="editor-head" data-testid="editor-head" data-save-state={saved}>
      <div className="editor-head-row">
        <DeviceName device={device} />
        <span className="visually-hidden" data-testid="connection" data-tone={status.tone}>
          {name}: {status.words}
        </span>
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
              onClick={() => {
                if (adding) {
                  store.closeAdd();
                  return;
                }
                store.openAdd();
                openDetails();
              }}
              data-testid="add-tile"
            >
              <Plus size={16} strokeWidth={2} aria-hidden />
              <span className="add-tile-words">Add Tile</span>
            </button>
            <SendButton store={store} name={name} live={status.live} />
            <DetailsToggle store={store} />
          </>
        )}
      </div>
    </header>
  );
}
