// Brandfetch Brand Search, called live from the UI with the user's own client ID.
// Brandfetch's terms: results are fetched live, never cached, altered or stored, and the logo
// URLs are hotlinked. So hits live only in component state, requests skip the HTTP cache, and
// nothing here touches storage.

export const BRANDFETCH_API = 'https://api.brandfetch.io';
export const SEARCH_DELAY_MS = 300;
export const SEARCH_LIMIT = 5;
const ICON_HOSTS: ReadonlySet<string> = new Set(['cdn.brandfetch.io', 'asset.brandfetch.io']);

export interface BrandHit {
  /** The brand's name, or null when Brandfetch has none. Shown, never stored. */
  name: string | null;
  domain: string;
  brandId: string;
  /** A hotlinked image URL on a Brandfetch host, or null. */
  icon: string | null;
}

export type BrandSearchProblem = 'refused' | 'rate_limited' | 'bad_query' | 'unavailable' | 'offline';

export type BrandSearchState =
  | { kind: 'idle' }
  | { kind: 'loading'; query: string; hits: readonly BrandHit[] }
  | { kind: 'done'; query: string; hits: readonly BrandHit[] }
  | { kind: 'failed'; query: string; problem: BrandSearchProblem };

export class BrandSearchError extends Error {
  readonly problem: BrandSearchProblem;
  constructor(problem: BrandSearchProblem) {
    super(`Brandfetch search failed: ${problem}`);
    this.name = 'BrandSearchError';
    this.problem = problem;
  }
}

export function searchUrl(clientId: string, query: string): string {
  const url = new URL(`/v2/search/${encodeURIComponent(query)}`, BRANDFETCH_API);
  url.searchParams.set('c', clientId);
  url.searchParams.set('limit', String(SEARCH_LIMIT));
  return url.href;
}

function iconOf(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  try {
    const url = new URL(value);
    return url.protocol === 'https:' && ICON_HOSTS.has(url.hostname) && !url.username && !url.password ? url.href : null;
  } catch {
    return null;
  }
}

const text = (value: unknown): string | null => (typeof value === 'string' && value.trim() ? value : null);

/** Keeps well-formed hits only, at most five. */
export function parseHits(body: unknown): BrandHit[] {
  if (!Array.isArray(body)) return [];
  return body
    .flatMap((item: unknown): BrandHit[] => {
      if (typeof item !== 'object' || item === null) return [];
      const record: Partial<Record<string, unknown>> = Object.fromEntries(Object.entries(item));
      const domain = text(record.domain);
      const brandId = text(record.brandId);
      if (!domain || !brandId) return [];
      return [{ name: text(record.name), domain, brandId, icon: iconOf(record.icon) }];
    })
    .slice(0, SEARCH_LIMIT);
}

function problemOf(status: number): BrandSearchProblem {
  if (status === 400) return 'bad_query';
  if (status === 401 || status === 403) return 'refused';
  if (status === 429) return 'rate_limited';
  return 'unavailable';
}

export async function searchBrands(clientId: string, query: string, signal: AbortSignal, send: typeof fetch = fetch): Promise<BrandHit[]> {
  let response: Response;
  try {
    // The page sends no Referer by default; Brandfetch is told only which app searches.
    response = await send(searchUrl(clientId, query), { signal, cache: 'no-store', credentials: 'omit', referrerPolicy: 'origin' });
  } catch (error) {
    if (signal.aborted) throw error;
    throw new BrandSearchError('offline');
  }
  if (!response.ok) throw new BrandSearchError(problemOf(response.status));
  let body: unknown;
  try {
    body = await response.json();
  } catch (error) {
    if (signal.aborted) throw error;
    throw new BrandSearchError('unavailable');
  }
  return parseHits(body);
}

export const SEARCH_MESSAGES: Readonly<Record<BrandSearchProblem, string>> = {
  refused: 'Brandfetch refused the client ID. Check it in Settings, Brandfetch logos.',
  rate_limited: 'Too many Brandfetch searches for now. Wait a few minutes, then retry.',
  bad_query: "Brandfetch couldn't search for that. Try a shorter brand name.",
  unavailable: "Brandfetch couldn't answer. Retry in a moment.",
  offline: "Can't reach Brandfetch. Check the internet connection, then retry.",
};

export interface BrandSearchOptions {
  clientId: string;
  onChange(state: BrandSearchState): void;
  delayMs?: number;
  send?: typeof fetch;
}

export interface BrandSearch {
  type(query: string): void;
  retry(): void;
  close(): void;
}

/** Debounces keystrokes, aborts the search a newer one replaces, and keeps no history. */
export function createBrandSearch({ clientId, onChange, delayMs = SEARCH_DELAY_MS, send }: BrandSearchOptions): BrandSearch {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let running: AbortController | null = null;
  let query = '';
  let shown: readonly BrandHit[] = [];
  let closed = false;

  const stop = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
    running?.abort();
    running = null;
  };

  const run = () => {
    timer = null;
    const asked = query;
    const controller = new AbortController();
    running = controller;
    searchBrands(clientId, asked, controller.signal, send).then(
      (hits) => {
        if (running !== controller) return;
        running = null;
        shown = hits;
        onChange({ kind: 'done', query: asked, hits });
      },
      (error: unknown) => {
        if (running !== controller || controller.signal.aborted) return;
        running = null;
        shown = [];
        onChange({ kind: 'failed', query: asked, problem: error instanceof BrandSearchError ? error.problem : 'unavailable' });
      },
    );
  };

  return {
    type(next) {
      if (closed) return;
      const trimmed = next.trim();
      if (trimmed === query) return;
      stop();
      query = trimmed;
      if (!query) {
        shown = [];
        onChange({ kind: 'idle' });
        return;
      }
      onChange({ kind: 'loading', query, hits: shown });
      timer = setTimeout(run, delayMs);
    },
    retry() {
      if (closed || !query) return;
      stop();
      onChange({ kind: 'loading', query, hits: shown });
      run();
    },
    close() {
      closed = true;
      stop();
      shown = [];
    },
  };
}
