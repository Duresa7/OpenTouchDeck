import { createContext, useContext } from 'react';
import { useStore } from 'zustand';
import { createStore } from 'zustand/vanilla';
import type { OpenTouchDeckBridge, UpdateCheck, UpdateDownload, UpdateInstall } from '../lib/companion/session.ts';

export type UpdatesBridge = NonNullable<OpenTouchDeckBridge['updates']>;
export type UpdateOffer = Extract<UpdateCheck, { state: 'available' }>;
export type FailedReason = 'download' | 'signature' | 'offline' | 'installer';

export type UpdateStep =
  | { kind: 'offer' }
  | { kind: 'downloading'; downloaded: number; total: number | null }
  | { kind: 'ready' }
  | { kind: 'busy'; device: string | null }
  | { kind: 'installing' }
  | { kind: 'failed'; reason: FailedReason };

export type CheckState = 'idle' | 'checking' | 'current' | 'found' | 'disabled' | 'offline' | 'failed';

export interface UpdatesState {
  /** False in a browser tab, which has no updater to ask. */
  supported: boolean;
  check: CheckState;
  update: { offer: UpdateOffer; step: UpdateStep } | null;
  /** A 2.x release this PC can't run because it needs Windows 11, from the last check that found nothing newer. */
  needsWindows11: string | null;
  /** The version Later was chosen for. */
  laterFor: string | null;
  dialogOpen: boolean;
  /** Checks now, or joins the check already running. */
  checkForUpdates(): Promise<void>;
  download(): Promise<void>;
  install(): Promise<void>;
  retry(): Promise<void>;
  later(): void;
  setDialogOpen(open: boolean): void;
}

export const CHECK_EVERY_MS = 6 * 60 * 60 * 1000;

const downloadable = (step: UpdateStep) => step.kind === 'offer' || (step.kind === 'failed' && step.reason !== 'installer');
const installable = (step: UpdateStep) => step.kind === 'ready' || step.kind === 'busy' || (step.kind === 'failed' && step.reason === 'installer');

function afterCheck(state: UpdatesState, found: UpdateCheck): Partial<UpdatesState> {
  const { update } = state;
  switch (found.state) {
    case 'available': {
      if (update && !downloadable(update.step)) return { check: 'found', needsWindows11: null };
      const step = update?.offer.version === found.version ? update.step : { kind: 'offer' as const };
      return { check: 'found', needsWindows11: null, update: { offer: found, step } };
    }
    case 'none':
    case 'disabled': {
      const keep = found.state === 'none' && update && !downloadable(update.step);
      const needsWindows11 = found.state === 'none' ? (found.needsWindows11 ?? null) : null;
      return { check: found.state === 'none' ? 'current' : 'disabled', needsWindows11, ...(keep ? {} : { update: null, dialogOpen: false }) };
    }
    case 'offline':
    case 'failed':
      return { check: found.state };
    default: {
      const _exhaustive: never = found;
      return _exhaustive;
    }
  }
}

function downloaded(result: UpdateDownload): UpdateStep {
  switch (result.state) {
    case 'ready':
      return { kind: 'ready' };
    case 'offline':
      return { kind: 'failed', reason: 'offline' };
    case 'failed':
      return { kind: 'failed', reason: result.reason };
    default: {
      const _exhaustive: never = result;
      return _exhaustive;
    }
  }
}

function installFailed(result: UpdateInstall): UpdateStep {
  switch (result.reason) {
    case 'busy':
      return { kind: 'busy', device: result.device };
    case 'installer':
      return { kind: 'failed', reason: 'installer' };
    default: {
      const _exhaustive: never = result;
      return _exhaustive;
    }
  }
}

function settle<T>(call: () => Promise<T>, failure: T): Promise<T> {
  return Promise.resolve()
    .then(call)
    .catch(() => failure);
}

export function createUpdatesStore(bridge: UpdatesBridge | undefined) {
  let checking: Promise<void> | null = null;

  return createStore<UpdatesState>()((set, get) => {
    const toStep = (step: UpdateStep) => set((state) => (state.update ? { update: { ...state.update, step } } : state));

    return {
      supported: bridge !== undefined,
      check: 'idle',
      update: null,
      needsWindows11: null,
      laterFor: null,
      dialogOpen: false,

      checkForUpdates() {
        if (!bridge) return Promise.resolve();
        checking ??= (async () => {
          set({ check: 'checking' });
          const found = await settle(() => bridge.check(), { state: 'failed' });
          checking = null;
          set((state) => afterCheck(state, found));
        })();
        return checking;
      },

      async download() {
        const { update } = get();
        if (!bridge || !update || !downloadable(update.step)) return;
        toStep({ kind: 'downloading', downloaded: 0, total: null });
        const result = await settle(
          () =>
            bridge.download(({ downloaded, total }) => {
              if (get().update?.step.kind === 'downloading') toStep({ kind: 'downloading', downloaded, total });
            }),
          { state: 'failed', reason: 'download' },
        );
        toStep(downloaded(result));
      },

      async install() {
        const { update } = get();
        if (!bridge || !update || !installable(update.step)) return;
        toStep({ kind: 'installing' });
        const result = await settle(() => bridge.install(), { state: 'failed', reason: 'installer' });
        toStep(installFailed(result));
      },

      retry() {
        const step = get().update?.step;
        return step && installable(step) ? get().install() : get().download();
      },

      later: () => set((state) => ({ dialogOpen: false, laterFor: state.update?.offer.version ?? state.laterFor })),
      setDialogOpen: (dialogOpen) => set((state) => ({ dialogOpen: dialogOpen && state.update !== null })),
    };
  });
}

export type UpdatesStore = ReturnType<typeof createUpdatesStore>;

/** Checks once now, then every `everyMs` until the returned function stops it. */
export function watchForUpdates(store: UpdatesStore, { everyMs = CHECK_EVERY_MS } = {}): () => void {
  if (!store.getState().supported) return () => {};
  void store.getState().checkForUpdates();
  const timer = setInterval(() => void store.getState().checkForUpdates(), everyMs);
  return () => clearInterval(timer);
}

export const UpdatesContext = createContext<UpdatesStore | null>(null);

export function useUpdates<T>(select: (state: UpdatesState) => T): T {
  const store = useContext(UpdatesContext);
  if (!store) throw new Error('useUpdates needs an UpdatesContext provider');
  return useStore(store, select);
}

export function useCheckForUpdates() {
  const supported = useUpdates((s) => s.supported);
  const check = useUpdates((s) => s.check);
  const checkForUpdates = useUpdates((s) => s.checkForUpdates);
  return { supported, check, checkForUpdates };
}
