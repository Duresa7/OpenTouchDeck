import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type RefObject } from 'react';
import clsx from 'clsx';
import { Switch } from '@base-ui/react/switch';
import { Tooltip } from '@base-ui/react/tooltip';
import type { DeviceSummary } from '../lib/companion/api.ts';
import { useCompanion } from '../lib/companion/connection.ts';
import { useDevices } from '../lib/companion/devices.ts';
import type { EditorStore } from '../lib/editor/store.ts';
import { needsFirmware, tileKind } from '../lib/device/needs-firmware.ts';
import { statusOf } from './home-model.ts';
import { ControlNoticeBar } from './layout/Controls.tsx';
import { useOfferedFirmware } from './layout/device.ts';
import { DeviceCanvas, deviceAspect } from './layout/DeviceCanvas.tsx';
import { editorFor, useEditorState } from './layout/editor.ts';
import { EditorHeader } from './layout/EditorHeader.tsx';
import { exportLayout, importLayout } from './layout/files.ts';
import { Inspector } from './layout/Inspector.tsx';
import { InspectorResizer, useInspectorWidth } from './layout/InspectorResizer.tsx';
import { PageTabs } from './layout/PageTabs.tsx';
import { ReplaceDialog } from './layout/ReplaceDialog.tsx';
import { StartLayout } from './layout/StartLayout.tsx';
import { StatusLine } from './layout/StatusLine.tsx';
import { useUi } from '../state/ui.ts';
import '../styles/layout.css';

const LONG_MIN = 420;
const LONG_MAX = 680;
/** With the Details panel hidden, the Device may grow into the room it leaves. */
const LONG_WIDE = 860;
/** Below 800px the details panel can't give way, so the Device may shrink past LONG_MIN rather than overflow. */
const LONG_NARROW = 240;

const editable = 'input, textarea, select, [contenteditable], [role="dialog"], [role="alertdialog"], [role="menu"]';

function useEditorKeys(store: EditorStore) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.altKey) return;
      const target = event.target instanceof HTMLElement ? event.target : null;
      if (target?.closest(editable)) return;
      const state = store.getState();
      const layout = state.drafts[state.orientation]?.layout;
      if (!layout) return;
      if (event.key === 'p' || event.key === 'P') {
        store.setMode(state.mode === 'edit' ? 'preview' : 'edit');
      } else if (state.mode === 'preview' && (event.key === 'ArrowLeft' || event.key === 'ArrowRight')) {
        if (target && target !== document.body && !target.closest('.canvas')) return;
        event.preventDefault();
        const index = layout.pages.findIndex((p) => p.id === state.page);
        const next = layout.pages[index + (event.key === 'ArrowLeft' ? -1 : 1)];
        if (next) store.selectPage(next.id);
      } else if (event.key === 'Escape') {
        if (state.adding) store.closeAdd();
        else if (state.selected) store.select(null);
      } else if ((event.key === 'Delete' || event.key === 'Backspace') && state.mode === 'edit' && state.selected) {
        event.preventDefault();
        store.clearTile(state.selected);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [store]);
}

interface Around {
  head: RefObject<HTMLDivElement | null>;
  foot: RefObject<HTMLDivElement | null>;
}

interface Bounds {
  floor: number;
  ceiling: number;
}

function useDeviceLong(canvas: RefObject<HTMLDivElement | null>, { head, foot }: Around, portrait: boolean, drawn: boolean, { floor, ceiling }: Bounds) {
  const [long, setLong] = useState(LONG_MAX);
  useLayoutEffect(() => {
    const el = canvas.current;
    if (!el) return;
    const fit = () => {
      const style = getComputedStyle(el);
      const aspect = deviceAspect();
      const gap = Number.parseFloat(style.rowGap);
      const wanted = (part: HTMLDivElement) => Math.max(part.offsetHeight, ...Array.from(part.children, (child) => child.scrollHeight));
      const taken = (part: HTMLDivElement | null) => (part?.offsetHeight ? wanted(part) + gap : 0);
      const w = el.clientWidth - Number.parseFloat(style.paddingLeft) - Number.parseFloat(style.paddingRight);
      const h = el.clientHeight - Number.parseFloat(style.paddingTop) - Number.parseFloat(style.paddingBottom) - taken(head.current) - taken(foot.current);
      const fitted = portrait ? Math.min(h, w / aspect) : Math.min(w, h / aspect);
      setLong(Math.round(Math.min(ceiling, Math.max(floor, fitted))));
    };
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(el);
    if (head.current) observer.observe(head.current);
    if (foot.current) observer.observe(foot.current);
    return () => observer.disconnect();
  }, [canvas, head, foot, portrait, drawn, floor, ceiling]);
  return long;
}

/** Sets the dock's inset without the slide, as screens never animate when you switch between them. */
function snapDock(set: (root: HTMLElement) => void) {
  const root = document.documentElement;
  root.setAttribute('data-dock-snap', '');
  set(root);
  const dock = document.querySelector('.nav-dock');
  if (dock) getComputedStyle(dock).left;
  root.removeAttribute('data-dock-snap');
}

/**
 * Tells the nav dock how much of the window's right side the Details panel takes, so it centres over the canvas. The
 * dock slides with the panel as it opens and closes, and jumps when the editor opens or leaves.
 */
function useDockInset(width: number) {
  useLayoutEffect(() => {
    snapDock((root) => root.style.setProperty('--dock-inset', `${width}px`));
    return () => snapDock((root) => root.style.removeProperty('--dock-inset'));
    // Only on mount and unmount: the width it starts with, then the effect below keeps it current.
  }, []);
  useLayoutEffect(() => {
    document.documentElement.style.setProperty('--dock-inset', `${width}px`);
  }, [width]);
}

function Editor({ store, device }: { store: EditorStore; device: DeviceSummary }) {
  const name = device.name ?? 'Unnamed Device';
  const connected = statusOf(device.link.state).live;
  const layout = useEditorState(store, (s) => s.drafts[s.orientation]?.layout ?? null);
  const page = useEditorState(store, (s) => s.page);
  const mode = useEditorState(store, (s) => s.mode);
  const selected = useEditorState(store, (s) => s.selected);
  const adding = useEditorState(store, (s) => s.adding);
  const [actualSize, setActualSize] = useState(false);
  const canvas = useRef<HTMLDivElement>(null);
  const head = useRef<HTMLDivElement>(null);
  const foot = useRef<HTMLDivElement>(null);
  const picker = useRef<HTMLInputElement>(null);
  const editor = useRef<HTMLElement>(null);
  const inspector = useInspectorWidth(editor);
  const detailsOpen = useUi((s) => s.detailsOpen);
  const long = useDeviceLong(canvas, { head, foot }, layout?.orientation === 'portrait', layout !== null, {
    floor: inspector.limits.fixed ? LONG_NARROW : LONG_MIN,
    ceiling: detailsOpen ? LONG_MAX : LONG_WIDE,
  });
  const offered = useOfferedFirmware(device);
  useEditorKeys(store);
  useDockInset(layout !== null && detailsOpen ? inspector.width : 0);

  const current = layout?.pages.find((p) => p.id === page) ?? layout?.pages[0];
  const preview = mode === 'preview';
  const openPicker = () => picker.current?.click();

  return (
    <main
      ref={editor}
      className={clsx('editor', !layout && 'is-empty', !detailsOpen && 'is-details-hidden')}
      style={{ '--inspector-w': `${inspector.width}px` } as CSSProperties}
      data-testid="editor"
      data-device-id={device.id}
      data-inspector-fixed={inspector.limits.fixed}
    >
      <h1 className="visually-hidden">Layout</h1>
      <EditorHeader store={store} device={device} onImport={openPicker} onExport={() => void exportLayout(store)} />
      <div ref={canvas} className={clsx('canvas', actualSize && preview && 'is-actual')}>
        <div ref={head} className="canvas-head">
          {!connected && (
            <p className="editor-banner" data-testid="disconnected">
              <strong>{name} isn't connected.</strong> Plug it in with a USB cable. You can still edit its Layout.
            </p>
          )}
          <ControlNoticeBar store={store} device={name} />
        </div>
        {layout && current ? (
          <>
            <DeviceCanvas
              store={store}
              layout={layout}
              pageId={current.id}
              mode={mode}
              selected={selected}
              adding={adding}
              long={long}
              actualSize={preview && actualSize}
              firmwareBadge={(tile) => needsFirmware(device, tileKind(tile), offered)}
              live={{ series: device.capabilities?.series ?? 0, stale: !connected }}
            />
            <div ref={foot} className="canvas-foot">
              <PageTabs store={store} layout={layout} page={current.id} />
              {preview && (
                <div className="preview-bar">
                  <p className="canvas-hint">Swipe or press ← and → to change Pages. Tap a Tile to see it react.</p>
                  <label className="switch-label">
                    <Switch.Root className="switch" checked={actualSize} onCheckedChange={setActualSize} data-testid="actual-size">
                      <Switch.Thumb className="switch-thumb" />
                    </Switch.Root>
                    Actual size
                  </label>
                </div>
              )}
              <StatusLine store={store} device={name} />
            </div>
          </>
        ) : (
          <>
            <StartLayout store={store} device={name} connected={connected} onImport={openPicker} />
            <StatusLine store={store} device={name} />
          </>
        )}
      </div>
      {layout && <Inspector store={store} layout={layout} device={name} />}
      {layout && detailsOpen && !inspector.limits.fixed && <InspectorResizer editor={editor} width={inspector.width} limits={inspector.limits} onWidth={inspector.choose} />}
      <ReplaceDialog store={store} device={name} />
      <input
        ref={picker}
        type="file"
        accept=".json,application/json"
        hidden
        onChange={(event) => {
          const file = event.currentTarget.files?.[0];
          event.currentTarget.value = '';
          if (file) void importLayout(store, file);
        }}
        data-testid="import-file"
      />
    </main>
  );
}

export function Layout() {
  const companion = useCompanion();
  const store = editorFor(companion);
  const deviceId = useUi((s) => s.deviceId);
  const device = useDevices((s) => (deviceId ? s.devices.find((d) => d.id === deviceId) : s.devices.find((d) => d.name !== null)));
  const id = device?.id;
  const name = device?.name;
  const orientation = device?.orientation;

  useLayoutEffect(() => {
    if (id && name) store.open({ id, name }, orientation === 'portrait' ? 'portrait' : 'landscape');
  }, [store, id, name, orientation]);

  if (!device) {
    return (
      <main className="screen" data-device-id={deviceId}>
        <header className="page-head">
          <h1 className="page-title">Layout</h1>
        </header>
        <div className="empty empty-start">
          <h2 className="empty-title">Plug in your Device</h2>
          <p className="empty-text">Connect an Open TouchDeck to this PC with a USB data cable.</p>
        </div>
      </main>
    );
  }
  return (
    <Tooltip.Provider delay={500} closeDelay={0}>
      <Editor store={store} device={device} />
    </Tooltip.Provider>
  );
}
