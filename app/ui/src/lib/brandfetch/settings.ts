// The Brandfetch settings the Companion keeps: the client ID (not a secret) and whether an API
// key is stored (the key itself never comes back). One store per Companion, shared by the
// Settings section, the icon picker and the editor.
import { createStore, type StoreApi } from 'zustand/vanilla';
import type { BrandfetchStatus } from '../companion/api.ts';
import type { Companion } from '../companion/client.ts';
import { isApiError } from '../companion/errors.ts';
import { keychainMissing, platformOf } from '../companion/keychain.ts';
import { UNREACHABLE } from '../icons/pick.ts';

export const DEVELOPER_PORTAL = 'https://developers.brandfetch.com/register';

/** A sentence from the copy guide, and the Companion's own words for Details. */
export interface BrandfetchProblem {
  text: string;
  detail: string | null;
}

/** Where the last change stands. */
export type BrandfetchStep =
  | { kind: 'idle' }
  | { kind: 'busy' }
  | { kind: 'failed'; problem: BrandfetchProblem }
  | { kind: 'done'; notice: string };

export type BrandfetchView =
  | { kind: 'loading' }
  | { kind: 'ready'; status: BrandfetchStatus; step: BrandfetchStep }
  | { kind: 'unreadable'; problem: string };

export interface BrandfetchState {
  view: BrandfetchView;
  load(): Promise<void>;
  saveClientId(id: string): Promise<boolean>;
  forgetClientId(): Promise<boolean>;
  saveKey(value: string): Promise<boolean>;
  forgetKey(): Promise<boolean>;
  /** One Brand API call, on request only: saving never calls Brandfetch. */
  testKey(): Promise<boolean>;
}

const READ_PROBLEM = "Couldn't read the Brandfetch settings. Restart Open TouchDeck, then retry.";
const BAD_CLIENT_ID = "That doesn't look like a client ID. Copy it from the Brandfetch developer portal.";
const BAD_KEY = "That doesn't look like an API key. Copy it from the Brandfetch developer portal.";
const SAVE_PROBLEM = "Couldn't save the Brandfetch settings. Retry, or restart Open TouchDeck.";
const NO_KEY = 'Save an API key first.';
const KEY_REFUSED = 'Brandfetch refused the key. Replace it with the API key from the Brandfetch developer portal.';
const QUOTA = 'Your Brandfetch API requests are used up. Check your plan in the Brandfetch developer portal.';
const NO_ANSWER = "Brandfetch didn't answer. Check the internet connection, then retry.";
const KEY_WORKS = 'Brandfetch accepted the key.';

const IDLE: BrandfetchStep = { kind: 'idle' };
const BUSY: BrandfetchStep = { kind: 'busy' };

/** True while a change is on its way to the Companion. */
export const isBusy = (view: BrandfetchView) => view.kind === 'ready' && view.step.kind === 'busy';

type Companionish = Pick<Companion, 'brandfetch' | 'system'>;

export function createBrandfetchStore(companion: Companionish) {
  const problem = async (error: unknown, text: (error: unknown) => string | null): Promise<BrandfetchProblem> => {
    const detail = isApiError(error) ? error.message : null;
    if (isApiError(error, 'client.unreachable')) return { text: UNREACHABLE, detail };
    // The Companion names no platform; this says which keychain and how to start it.
    if (isApiError(error, 'brandfetch.no_keychain')) return { text: keychainMissing(await platformOf(companion)), detail };
    return { text: text(error) ?? SAVE_PROBLEM, detail };
  };
  const saving = (invalid: string) => (error: unknown) => (isApiError(error, 'request.invalid') ? invalid : null);
  const testing = (error: unknown) => {
    if (isApiError(error, 'brandfetch.no_key')) return NO_KEY;
    if (isApiError(error, 'brandfetch.key_refused')) return KEY_REFUSED;
    if (isApiError(error, 'brandfetch.quota')) return QUOTA;
    return NO_ANSWER;
  };

  return createStore<BrandfetchState>()((set, get) => {
    const change = async (send: () => Promise<BrandfetchStatus>, explain: (error: unknown) => string | null, done: BrandfetchStep = IDLE): Promise<boolean> => {
      const view = get().view;
      if (view.kind !== 'ready' || view.step.kind === 'busy') return false;
      set({ view: { ...view, step: BUSY } });
      try {
        set({ view: { kind: 'ready', status: await send(), step: done } });
        return true;
      } catch (error) {
        const failed: BrandfetchStep = { kind: 'failed', problem: await problem(error, explain) };
        const now = get().view;
        set({ view: { kind: 'ready', status: now.kind === 'ready' ? now.status : view.status, step: failed } });
        return false;
      }
    };
    return {
      view: { kind: 'loading' },
      async load() {
        try {
          const status = await companion.brandfetch.get();
          const now = get().view;
          set({ view: { kind: 'ready', status, step: now.kind === 'ready' ? now.step : IDLE } });
        } catch (error) {
          if (get().view.kind !== 'ready') set({ view: { kind: 'unreadable', problem: isApiError(error, 'client.unreachable') ? UNREACHABLE : READ_PROBLEM } });
        }
      },
      saveClientId: (id) => change(() => companion.brandfetch.client_id_put({}, null, { client_id: id.trim() }), saving(BAD_CLIENT_ID)),
      forgetClientId: () => change(() => companion.brandfetch.client_id_delete(), saving(BAD_CLIENT_ID)),
      saveKey: (value) => change(() => companion.brandfetch.key_put({}, null, { value }), saving(BAD_KEY)),
      forgetKey: () => change(() => companion.brandfetch.key_delete(), saving(BAD_KEY)),
      testKey: () => change(() => companion.brandfetch.key_test(), testing, { kind: 'done', notice: KEY_WORKS }),
    };
  });
}

export type BrandfetchStore = StoreApi<BrandfetchState>;

const stores = new WeakMap<object, BrandfetchStore>();

/** The shared store for this Companion client. */
export function brandfetchStore(companion: Companionish): BrandfetchStore {
  let store = stores.get(companion);
  if (!store) {
    store = createBrandfetchStore(companion);
    stores.set(companion, store);
  }
  return store;
}

/** The client ID search runs with, or null while Brandfetch is off or unread. */
export function clientIdOf(view: BrandfetchView): string | null {
  return view.kind === 'ready' ? view.status.client_id : null;
}

/** Each Asset a refresh replaced, and the Asset that replaced it. Empty until read. */
export function replacementsOf(view: BrandfetchView): Readonly<Partial<Record<string, string>>> {
  return view.kind === 'ready' ? view.status.replaced : NONE;
}
const NONE: Readonly<Partial<Record<string, string>>> = Object.freeze({});

/** "3 logos; 1 needs a refresh", or null when there are none. */
export function logoCount(status: Pick<BrandfetchStatus, 'logos' | 'need_refresh'>): string | null {
  if (status.logos === 0) return null;
  const logos = `${status.logos} ${status.logos === 1 ? 'logo' : 'logos'}`;
  if (status.need_refresh === 0) return `${logos}, all up to date`;
  return `${logos}; ${status.need_refresh} ${status.need_refresh === 1 ? 'needs' : 'need'} a refresh`;
}
