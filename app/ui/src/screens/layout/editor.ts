import { useMemo } from 'react';
import { useStore } from 'zustand';
import { brandfetchStore, replacementsOf } from '../../lib/brandfetch/settings.ts';
import type { Companion } from '../../lib/companion/client.ts';
import { useCompanion } from '../../lib/companion/connection.ts';
import { devicesStore, useDevices } from '../../lib/companion/devices.ts';
import { jobsStore, waitForJob } from '../../lib/companion/jobs.ts';
import { showsKind, type TileKind } from '../../lib/device/needs-firmware.ts';
import { offersGif } from '../../lib/editor/gifs.ts';
import { createGifTiles, type GifTiles, type GifTilesState } from '../../lib/editor/gifs.ts';
import { controlsStore, type ControlsStore } from '../../lib/device/controls.ts';
import { createEditorStore, effectiveTileEdit, type EditorDevice, type EditorState, type EditorStore } from '../../lib/editor/store.ts';
import type { Orientation } from '../../lib/layout/model.ts';

const stores = new WeakMap<Companion, EditorStore>();
const gifStores = new WeakMap<Companion, GifTiles>();

function storage() {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export function watchSends(editor: EditorStore, controls: ControlsStore): () => void {
  return editor.subscribe((next, previous) => {
    const device = next.device?.id;
    if (!device || next.busy === previous.busy) return;
    if (next.busy === 'push') controls.getState().sending(device, next.orientation);
    else if (previous.busy === 'push') controls.getState().sent(device, next.status?.event === 'pushed' ? 'succeeded' : 'failed');
  });
}

/**
 * Applies the logos a Brandfetch refresh replaced to the editor's drafts. The Companion never
 * changes a Layout itself, so the new logo reaches the Device on the next Send to Device. The
 * list is read when the editor starts and again whenever this window comes back into view.
 */
export function watchReplacements(editor: EditorStore, companion: Companion): () => void {
  const brandfetch = brandfetchStore(companion);
  let last = replacementsOf(brandfetch.getState().view);
  editor.replaceAssets(last);
  const stop = brandfetch.subscribe(({ view }) => {
    const next = replacementsOf(view);
    if (next === last) return;
    last = next;
    editor.replaceAssets(next);
  });
  void brandfetch.getState().load();
  if (typeof document === 'undefined') return stop;
  const reread = () => {
    if (document.visibilityState === 'visible') void brandfetch.getState().load();
  };
  document.addEventListener('visibilitychange', reread);
  return () => {
    stop();
    document.removeEventListener('visibilitychange', reread);
  };
}

export function editorFor(companion: Companion): EditorStore {
  let store = stores.get(companion);
  if (!store) {
    store = createEditorStore({ companion, storage: storage(), wait: (job) => waitForJob(jobsStore, companion, job) });
    watchSends(store, controlsStore);
    watchReplacements(store, companion);
    stores.set(companion, store);
  }
  return store;
}

export function reviewLayout(store: EditorStore, device: EditorDevice, orientation: Orientation) {
  store.open(device, orientation);
  store.setOrientation(orientation);
  void store.load();
}

/** The GIF Tiles of the editor that `editorFor` gives for this Companion. */
export function gifTilesFor(companion: Companion): GifTiles {
  let gifs = gifStores.get(companion);
  if (!gifs) {
    gifs = createGifTiles({
      editor: editorFor(companion),
      companion,
      jobs: jobsStore,
      devices: devicesStore,
      wait: (job, signal) => waitForJob<'assets.render'>(jobsStore, companion, job, { signal }),
    });
    gifStores.set(companion, gifs);
  }
  return gifs;
}

export function useGifTiles(): GifTiles {
  return gifTilesFor(useCompanion());
}

export function useGifState<T>(gifs: GifTiles, select: (state: GifTilesState) => T): T {
  return useStore(gifs, select);
}

/** Whether the open Device takes animated GIF Tiles. */
export function useDeviceOffersGif(store: EditorStore): boolean {
  const id = useEditorState(store, (s) => s.device?.id);
  return useDevices((s) => offersGif(s.devices.find((d) => d.id === id)?.capabilities));
}

export function useEditorState<T>(store: EditorStore, select: (state: EditorState) => T): T {
  return useStore(store, select);
}

/** A Tile's effective edit, re-rendering only when its draft, its refusals or the orientation change. */
export function useEffectiveTileEdit(store: EditorStore, device: string, tileId: string) {
  const orientation = useEditorState(store, (s) => s.orientation);
  const draft = useEditorState(store, (s) => s.drafts[s.orientation]);
  const problems = useEditorState(store, (s) => s.invalid[s.orientation][tileId]);
  return useMemo(() => effectiveTileEdit(store.getState(), device, tileId), [store, orientation, draft, problems, device, tileId]);
}

/** Whether the open Device's Firmware draws this kind of Tile. */
export function useDeviceShows(store: EditorStore, kind: TileKind): boolean {
  const id = useEditorState(store, (s) => s.device?.id);
  return useDevices((s) => {
    const device = s.devices.find((d) => d.id === id);
    return device ? showsKind(device, kind) : false;
  });
}
