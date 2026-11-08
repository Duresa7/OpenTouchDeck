import { useStore } from 'zustand';
import { createStore } from 'zustand/vanilla';
import type { IntegrationSummary, IntegrationView } from '../companion/api.ts';
import type { Companion } from '../companion/client.ts';
import { integrationsStore, type IntegrationsStore } from '../companion/integrations.ts';

/** What flipping an Integration's switch does: turn it on or off, or open its setup first. */
export type Flip = 'enable' | 'disable' | 'set-up';

export function flipOf(summary: Pick<IntegrationSummary, 'status'>, on: boolean): Flip {
  if (!on) return 'disable';
  return summary.status.state === 'not_configured' ? 'set-up' : 'enable';
}

export interface SwitchView {
  checked: boolean;
  /** A write is on its way, or an enable waits for setup to finish. */
  busy: boolean;
  /** A write is on its way or queued, so the switch can't be flipped. An enable waiting for setup can still be undone. */
  locked: boolean;
}

/**
 * What the switch shows: the saved state, the state it's saving, or on while an enable waits for setup. `saving` is
 * the state being written, or null.
 */
export function switchView(enabled: boolean, saving: boolean | null, pending: boolean, writes = 0): SwitchView {
  const locked = saving !== null || writes > 0;
  if (saving !== null) return { checked: saving, busy: true, locked };
  if (pending && !enabled) return { checked: true, busy: true, locked };
  return { checked: enabled, busy: locked, locked };
}

/** A pending enable completes once a save has set the Integration up. */
export const completesPending = (summary: Pick<IntegrationSummary, 'enabled' | 'status'>, pending: boolean) =>
  pending && !summary.enabled && summary.status.state !== 'not_configured';

/** Whether an Integration has a settings page: config beyond its switch. */
export const hasSettings = (summary: Pick<IntegrationSummary, 'config_schema'>) => summary.config_schema.some((field) => field.name !== 'enabled');

export interface WritesState {
  /** The state each switch is writing, by Integration id. */
  saving: Partial<Record<string, boolean>>;
  /** How many writes each Integration has queued or running. */
  writes: Partial<Record<string, number>>;
}

/**
 * One queue per Integration for everything that writes it: its switch, its settings Save and its secrets. Writes run
 * one at a time in the order they were asked for, and share one busy state.
 */
export function createWrites(companion: Pick<Companion, 'integrations'>, integrations: IntegrationsStore = integrationsStore) {
  const store = createStore<WritesState>()(() => ({ saving: {}, writes: {} }));
  const tails = new Map<string, Promise<void>>();

  const count = (id: string, by: number) =>
    store.setState((s) => {
      const next = (s.writes[id] ?? 0) + by;
      const writes = { ...s.writes };
      if (next > 0) writes[id] = next;
      else delete writes[id];
      return { writes };
    });

  function run<T>(id: string, work: () => Promise<T>): Promise<T> {
    count(id, 1);
    const result = (tails.get(id) ?? Promise.resolve()).then(work);
    tails.set(
      id,
      result.then(
        () => undefined,
        () => undefined,
      ),
    );
    return result.finally(() => count(id, -1));
  }

  /** Turns the Integration on or off and nothing else. Null when the Companion refused or couldn't be reached. */
  async function setEnabled(id: string, on: boolean): Promise<IntegrationView | null> {
    store.setState((s) => ({ saving: { ...s.saving, [id]: on } }));
    try {
      const view = await run(id, () => companion.integrations.enabled_put({ integration: id }, null, { enabled: on }));
      integrations.getState().upsert(view.summary);
      return view;
    } catch {
      return null;
    } finally {
      store.setState((s) => {
        const saving = { ...s.saving };
        delete saving[id];
        return { saving };
      });
    }
  }

  return Object.assign(store, { run, setEnabled });
}

export type Writes = ReturnType<typeof createWrites>;

const shared = new WeakMap<object, Writes>();

/** The write queues for this Companion, shared by every screen. */
export function writesFor(companion: Pick<Companion, 'integrations'>): Writes {
  let writes = shared.get(companion);
  if (!writes) {
    writes = createWrites(companion);
    shared.set(companion, writes);
  }
  return writes;
}

export function useWrites<T>(writes: Writes, select: (state: WritesState) => T): T {
  return useStore(writes, select);
}
