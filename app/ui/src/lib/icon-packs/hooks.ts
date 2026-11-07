import { useEffect, useState } from 'react';
import { useStore } from 'zustand';
import type { PackStage } from '../companion/api.ts';
import type { Companion } from '../companion/client.ts';
import { jobsStore, waitForJob } from '../companion/jobs.ts';
import { createIconPacks, type IconPacks, type IconPacksState } from './store.ts';
import { createThumbnails, type Thumbnail, type Thumbnails } from './thumbnails.ts';

const packs = new WeakMap<Companion, IconPacks>();
const thumbnails = new WeakMap<Companion, Thumbnails>();

/** The SVG thumbnails of one Companion's pack icons, shared by every view. */
export function thumbnailsOf(companion: Companion): Thumbnails {
  let found = thumbnails.get(companion);
  if (!found) {
    found = createThumbnails({
      fetch: (prefix, name) => companion.icon_packs.thumbnail({ prefix, icon: `${name}.svg` }),
      toUrl: (blob) => URL.createObjectURL(blob),
      revoke: (url) => URL.revokeObjectURL(url),
    });
    thumbnails.set(companion, found);
  }
  return found;
}

/** One Companion's Icon packs, shared by Settings and the icon picker. */
export function iconPacksOf(companion: Companion): IconPacks {
  let found = packs.get(companion);
  if (!found) {
    found = createIconPacks({
      companion,
      wait: (job) => waitForJob<'icon_packs.install'>(jobsStore, companion, job),
      changed: (prefix) => thumbnailsOf(companion).forget(prefix),
    });
    packs.set(companion, found);
  }
  return found;
}

/** Reads the pack list once per mount and selects from it. */
export function useIconPacks<T>(companion: Companion, select: (state: IconPacksState) => T): T {
  const store = iconPacksOf(companion);
  useEffect(() => {
    void store.load();
  }, [store]);
  return useStore(store.store, select);
}

/** The stage an install Job last reported, or `null` before its first report. */
export function useInstallStage(job: string | null): PackStage | null {
  return useStore(jobsStore, (state) => {
    const progress: unknown = job === null ? null : state.jobs[job]?.progress;
    if (typeof progress !== 'object' || progress === null || !('stage' in progress)) return null;
    const stage = progress.stage;
    return stage === 'download' || stage === 'verify' || stage === 'extract' || stage === 'done' ? stage : null;
  });
}

/** A pack icon's thumbnail URL once it loads. */
export function usePackThumbnail(companion: Companion, prefix: string, name: string): Thumbnail | null {
  const cache = thumbnailsOf(companion);
  const [shown, setShown] = useState<Thumbnail | null>(() => cache.peek(prefix, name) ?? null);
  useEffect(() => {
    let live = true;
    const known = cache.peek(prefix, name);
    setShown(known ?? null);
    if (known === undefined)
      void cache.get(prefix, name).then((value) => {
        if (live) setShown(value);
      });
    return () => {
      live = false;
    };
  }, [cache, prefix, name]);
  return shown;
}
