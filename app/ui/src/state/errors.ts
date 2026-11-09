import { useStore } from 'zustand';
import { createStore } from 'zustand/vanilla';

/** An error the app showed, kept for the Errors page in Settings. */
export interface ErrorEntry {
  id: number;
  /** When it happened, in milliseconds since the epoch. */
  at: number;
  /** Where it happened, such as "Layout · Desk". */
  where: string;
  text: string;
  detail: string | null;
}

export interface ErrorsState {
  /** Newest first, at most {@link KEEP}. */
  entries: ErrorEntry[];
  report(entry: Omit<ErrorEntry, 'id' | 'at'>, at?: number): ErrorEntry;
  clear(): void;
}

/** How many errors the page keeps. Older ones drop off the end. */
export const KEEP = 50;

export function createErrorsStore() {
  let next = 1;
  return createStore<ErrorsState>()((set) => ({
    entries: [],
    report(entry, at = Date.now()) {
      const kept: ErrorEntry = { ...entry, id: next++, at };
      set((s) => ({ entries: [kept, ...s.entries].slice(0, KEEP) }));
      return kept;
    },
    clear: () => set({ entries: [] }),
  }));
}

export type ErrorsStore = ReturnType<typeof createErrorsStore>;

/** The window's error log. It lasts until the window reloads. */
export const errorsStore = createErrorsStore();

export function useErrors<T>(select: (state: ErrorsState) => T): T {
  return useStore(errorsStore, select);
}
