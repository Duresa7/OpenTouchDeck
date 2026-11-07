import { createStore, type StoreApi } from 'zustand/vanilla';
import type { Job, PackInfo } from '../companion/api.ts';
import type { Companion } from '../companion/client.ts';
import { toApiError } from '../companion/errors.ts';
import { importProblem, installProblem, LIST_FAILED, removeProblem } from './model.ts';

/** What is happening to one pack: its install Job, a remove, or the last failure. */
export type PackWork = { kind: 'installing'; job: string } | { kind: 'removing' } | { kind: 'failed'; text: string; detail: string };

export interface IconPacksState {
  packs: readonly PackInfo[] | null;
  load: 'idle' | 'loading' | 'ready' | 'failed';
  loadProblem: string | null;
  work: Readonly<Record<string, PackWork>>;
  importing: string | null;
  importProblem: { text: string; detail: string } | null;
}

export interface IconPacksDeps {
  companion: Pick<Companion, 'icon_packs' | 'jobs'>;
  /** Resolves with the install Job's result, or rejects with its error. */
  wait(job: Job): Promise<PackInfo>;
  /** Called after a pack is installed, removed or imported, so search and thumbnails refresh. */
  changed?(prefix: string): void;
}

export interface IconPacks {
  store: StoreApi<IconPacksState>;
  /** Reads the list. Concurrent calls share one request. */
  load(): Promise<void>;
  install(prefix: string): Promise<void>;
  remove(prefix: string): Promise<void>;
  importFile(file: Blob, name: string): Promise<void>;
}

const nameOf = (state: IconPacksState, prefix: string) => state.packs?.find((p) => p.prefix === prefix)?.name ?? prefix;

export function createIconPacks(deps: IconPacksDeps): IconPacks {
  const store = createStore<IconPacksState>()(() => ({ packs: null, load: 'idle', loadProblem: null, work: {}, importing: null, importProblem: null }));
  let reading: Promise<void> | null = null;

  const setWork = (prefix: string, work: PackWork | null) =>
    store.setState((state) => {
      const next = { ...state.work };
      if (work) next[prefix] = work;
      else delete next[prefix];
      return { work: next };
    });

  const put = (info: PackInfo) =>
    store.setState((state) => {
      const packs = state.packs ?? [];
      const at = packs.findIndex((p) => p.prefix === info.prefix);
      return { packs: at === -1 ? [...packs, info] : packs.map((p, i) => (i === at ? info : p)) };
    });

  const refresh = async () => {
    const packs = await deps.companion.icon_packs.list();
    store.setState({ packs, load: 'ready', loadProblem: null });
  };

  function load(): Promise<void> {
    reading ??= (async () => {
      if (store.getState().packs === null) store.setState({ load: 'loading' });
      try {
        await refresh();
      } catch {
        if (store.getState().packs === null) store.setState({ load: 'failed', loadProblem: LIST_FAILED });
      } finally {
        reading = null;
      }
    })();
    return reading;
  }

  async function install(prefix: string) {
    const work = store.getState().work[prefix];
    if (work?.kind === 'installing' || work?.kind === 'removing') return;
    const name = nameOf(store.getState(), prefix);
    try {
      const job = await deps.companion.icon_packs.install({ prefix });
      setWork(prefix, { kind: 'installing', job: job.id });
      put(await deps.wait(job));
      setWork(prefix, null);
      deps.changed?.(prefix);
    } catch (error) {
      setWork(prefix, { kind: 'failed', text: installProblem(error, name), detail: toApiError(error).message });
    }
  }

  async function remove(prefix: string) {
    if (store.getState().work[prefix]?.kind === 'installing') return;
    const name = nameOf(store.getState(), prefix);
    setWork(prefix, { kind: 'removing' });
    try {
      await deps.companion.icon_packs.remove({ prefix });
      setWork(prefix, null);
      deps.changed?.(prefix);
      await refresh().catch(() => {
        store.setState((state) => ({ packs: state.packs?.map((p) => (p.prefix === prefix ? { ...p, installed: false, size_bytes: null } : p)) ?? null }));
      });
    } catch (error) {
      setWork(prefix, { kind: 'failed', text: removeProblem(error, name), detail: toApiError(error).message });
    }
  }

  async function importFile(file: Blob, name: string) {
    if (store.getState().importing !== null) return;
    store.setState({ importing: name, importProblem: null });
    try {
      // The Companion reads the raw file as JSON, whatever type the picker gave it.
      const info = await deps.companion.icon_packs.import({}, null, new Blob([file], { type: 'application/json' }));
      put(info);
      store.setState({ importing: null });
      deps.changed?.(info.prefix);
    } catch (error) {
      store.setState({ importing: null, importProblem: { text: importProblem(error, name), detail: toApiError(error).message } });
    }
  }

  return { store, load, install, remove, importFile };
}
