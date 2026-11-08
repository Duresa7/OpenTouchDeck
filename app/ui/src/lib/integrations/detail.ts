import { createStore } from 'zustand/vanilla';
import type { IntegrationSummary, IntegrationView } from '../companion/api.ts';
import type { Companion } from '../companion/client.ts';
import { toApiError } from '../companion/errors.ts';
import { formOf, parseUsage, sameValues, serialize, validate, valuesOf, type FormField, type FormValues, type Usage } from './model.ts';

/** Refresh asks the hub at most this often, the same floor the Companion keeps. */
export const REFRESH_FLOOR_MS = 15_000;

export const refreshFloorText = (id: string) => id === 'custom' ? 'Checks are at least 1 s apart.' : FLOOR_TEXT;

export const FLOOR_TEXT = 'Checks are at least 15 s apart, because each one queries your AI services.';

export interface Notice {
  kind: 'ok' | 'info' | 'error';
  text: string;
  detail?: string;
  /** Names the outcome for tests. */
  event: string;
}

export interface SecretState {
  set: boolean;
  /** What `integrations.secret_put` answered, such as "Set". Never the value. */
  hint: string | null;
}

export interface DetailState {
  id: string;
  view: IntegrationView | null;
  usage: Usage | null;
  fields: FormField[];
  secretFields: FormField[];
  values: FormValues;
  /** The values as the Companion holds them, to tell whether the form has changes. */
  saved: FormValues;
  errors: Partial<Record<string, string>>;
  secrets: Partial<Record<string, SecretState>>;
  busy: 'load' | 'save' | 'secret' | 'refresh' | null;
  notice: Notice | null;
  /** When Refresh may ask the hub again. */
  refreshAfter: number;
}

export interface DetailDeps {
  companion: Pick<Companion, 'integrations'>;
  now?: () => number;
  /** The Integration's write queue, shared with its switch. Writes run straight away without one. */
  writes?: { run<T>(id: string, work: () => Promise<T>): Promise<T> };
  /** Hears the Integration's summary after each save of its settings or a secret, so a pending enable can finish. */
  onSummary?: (summary: IntegrationSummary) => void;
}

const RESTART = 'Retry, or restart Open TouchDeck.';

interface Issue {
  path?: string;
  message?: string;
}

function issuesOf(details: unknown): Issue[] {
  if (typeof details !== 'object' || details === null || !('issues' in details) || !Array.isArray(details.issues)) return [];
  return details.issues.filter((issue: unknown): issue is Issue => typeof issue === 'object' && issue !== null);
}

export function createDetailStore(id: string, deps: DetailDeps) {
  const now = deps.now ?? Date.now;
  const floorMs = id === 'custom' ? 1000 : REFRESH_FLOOR_MS;
  const floorText = refreshFloorText(id);
  const store = createStore<DetailState>()(() => ({
    id,
    view: null,
    usage: null,
    fields: [],
    secretFields: [],
    values: {},
    saved: {},
    errors: {},
    secrets: {},
    busy: null,
    notice: null,
    refreshAfter: 0,
  }));
  const { getState: state, setState: set } = store;
  const call = deps.companion.integrations;
  const write = <T>(work: () => Promise<T>) => (deps.writes ? deps.writes.run(id, work) : work());

  const dirty = () => !sameValues(state().fields, state().values, state().saved);

  function take(view: IntegrationView, { typed }: { typed: 'keep' | 'replace' }) {
    const { fields, secrets: secretFields } = formOf(id, view.summary.config_schema);
    const saved = valuesOf(fields, view.config);
    const keep = typed === 'keep' && state().view !== null && dirty();
    const secrets: Partial<Record<string, SecretState>> = {};
    for (const field of secretFields) {
      const stored = view.secrets[field.name]?.set === true;
      secrets[field.name] = { set: stored, hint: stored ? (state().secrets[field.name]?.hint ?? null) : null };
    }
    set({ view, usage: parseUsage(view), fields, secretFields, saved, secrets, values: keep ? state().values : saved, errors: keep ? state().errors : {} });
  }

  const label = (field: string) => (state().secretFields.find((f) => f.name === field)?.label ?? field).toLowerCase();

  return Object.assign(store, {
    /** Reads the view. A failure leaves the form as it was and says so. */
    async load(): Promise<void> {
      const first = state().view === null;
      if (first) set({ busy: 'load' });
      try {
        take(await call.get({ integration: id }), { typed: 'keep' });
      } catch (error) {
        set({ notice: { kind: 'error', text: `Couldn't load ${state().view?.summary.name ?? 'this Integration'}'s settings. ${RESTART}`, detail: toApiError(error).message, event: 'load-failed' } });
      } finally {
        if (first) set({ busy: null });
      }
    },

    setValue(name: string, value: string | boolean) {
      const errors = { ...state().errors };
      delete errors[name];
      set({ values: { ...state().values, [name]: value }, errors, notice: null });
    },

    dirty,

    /** Saves the settings. Says so when the Companion forgot a secret because the address moved to another origin. */
    async save(): Promise<void> {
      const { fields, values, busy } = state();
      if (busy) return;
      const errors = validate(fields, values);
      if (Object.keys(errors).length) {
        set({ errors, notice: null });
        return;
      }
      const before = state().secrets;
      set({ busy: 'save', notice: { kind: 'info', text: 'Saving…', event: 'saving' } });
      try {
        const view = await write(() => call.config_put({ integration: id }, null, serialize(fields, values)));
        take(view, { typed: 'replace' });
        deps.onSummary?.(view.summary);
        const forgotten = Object.keys(before).filter((field) => before[field]?.set && !state().secrets[field]?.set);
        set({
          notice: forgotten.length
            ? { kind: 'info', text: `Saved. The address changed, so the ${forgotten.map(label).join(' and ')} was forgotten. Enter it again.`, event: 'secret-forgotten' }
            : { kind: 'ok', text: 'Saved', event: 'saved' },
        });
      } catch (thrown) {
        const error = toApiError(thrown);
        if (error.code === 'integration.config_invalid') {
          const named: Partial<Record<string, string>> = {};
          for (const issue of issuesOf(error.details)) if (issue.path && fields.some((f) => f.name === issue.path)) named[issue.path] = issue.message ?? 'Check this field.';
          set({ errors: named, notice: { kind: 'error', text: "Couldn't save. Check the fields, then retry.", detail: issuesOf(error.details).map((i) => i.message).join('\n') || error.message, event: 'save-failed' } });
        } else {
          set({ notice: { kind: 'error', text: `Couldn't save. ${RESTART}`, detail: error.message, event: 'save-failed' } });
        }
      } finally {
        set({ busy: null });
      }
    },

    /** Writes a secret. The value goes to the Companion and nowhere else. */
    async putSecret({ field, value }: { field: string; value: string }): Promise<boolean> {
      if (state().busy) return false;
      if (!value.trim()) {
        set({ errors: { ...state().errors, [field]: `Enter the ${label(field)}.` } });
        return false;
      }
      set({ busy: 'secret', notice: { kind: 'info', text: 'Saving…', event: 'secret-saving' } });
      try {
        const { hint } = await write(() => call.secret_put({ integration: id, field }, null, { value }));
        const errors = { ...state().errors };
        delete errors[field];
        set({ errors, secrets: { ...state().secrets, [field]: { set: true, hint } }, notice: { kind: 'ok', text: 'Saved', event: 'secret-saved' } });
        try {
          const view = await call.get({ integration: id });
          take(view, { typed: 'keep' });
          deps.onSummary?.(view.summary);
        } catch {
          // The next status event brings the summary instead.
        }
        return true;
      } catch (error) {
        set({ notice: { kind: 'error', text: `Couldn't save the ${label(field)}. ${RESTART}`, detail: toApiError(error).message, event: 'secret-failed' } });
        return false;
      } finally {
        set({ busy: null });
      }
    },

    async forgetSecret(field: string): Promise<void> {
      if (state().busy) return;
      set({ busy: 'secret', notice: null });
      try {
        await write(() => call.secret_delete({ integration: id, field }));
        set({ secrets: { ...state().secrets, [field]: { set: false, hint: null } } });
      } catch (error) {
        set({ notice: { kind: 'error', text: `Couldn't forget the ${label(field)}. ${RESTART}`, detail: toApiError(error).message, event: 'forget-failed' } });
      } finally {
        set({ busy: null });
      }
    },

    /** Starts a check, at most once per floor. The Companion answers before the check ends. */
    async refresh(): Promise<void> {
      if (state().busy) return;
      if (now() < state().refreshAfter) {
        set({ notice: { kind: 'info', text: floorText, event: 'refresh-floor' } });
        return;
      }
      set({ busy: 'refresh', notice: null, refreshAfter: now() + floorMs });
      try {
        await call.refresh({ integration: id });
        take(await call.get({ integration: id }), { typed: 'keep' });
        set({ notice: { kind: 'ok', text: 'Check started', event: 'refreshed' } });
      } catch (thrown) {
        const error = toApiError(thrown);
        set({
          notice:
            error.code === 'limits.exceeded'
              ? { kind: 'info', text: floorText, detail: error.message, event: 'refresh-floor' }
              : { kind: 'error', text: `Couldn't check. ${RESTART}`, detail: error.message, event: 'refresh-failed' },
        });
      } finally {
        set({ busy: null });
      }
    },
  });
}

export type DetailStore = ReturnType<typeof createDetailStore>;
