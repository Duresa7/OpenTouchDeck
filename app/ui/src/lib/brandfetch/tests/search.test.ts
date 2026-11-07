import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createBrandSearch, parseHits, SEARCH_DELAY_MS, SEARCH_LIMIT, SEARCH_MESSAGES, searchUrl, type BrandSearchState } from '../search.ts';
import { SEARCH_HITS } from './fixtures.ts';

interface Asked {
  url: URL;
  init: RequestInit | undefined;
}

function harness(answer: (url: URL) => Response | Promise<Response> = () => Response.json(SEARCH_HITS)) {
  const asked: Asked[] = [];
  const states: BrandSearchState[] = [];
  const send: typeof fetch = (input, init) => {
    const url = new URL(input instanceof Request ? input.url : String(input));
    asked.push({ url, init });
    return Promise.resolve(answer(url));
  };
  const search = createBrandSearch({ clientId: 'test-client-id', onChange: (state) => states.push(state), send });
  return { search, asked, states };
}

describe('Brandfetch search', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('waits at least 300 ms after the last keystroke and sends one search', async () => {
    expect(SEARCH_DELAY_MS).toBeGreaterThanOrEqual(300);
    const { search, asked, states } = harness();
    for (const typed of ['e', 'ex', 'exa', 'example']) {
      search.type(typed);
      await vi.advanceTimersByTimeAsync(100);
    }
    expect(asked).toEqual([]);
    await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS - 101);
    expect(asked).toEqual([]);
    await vi.advanceTimersByTimeAsync(1);
    expect(asked).toHaveLength(1);
    const { url, init } = asked[0];
    expect(url.origin + url.pathname).toBe('https://api.brandfetch.io/v2/search/example');
    expect(url.searchParams.get('c')).toBe('test-client-id');
    expect(url.searchParams.get('limit')).toBe(String(SEARCH_LIMIT));
    expect(init?.cache).toBe('no-store');
    expect(init?.credentials).toBe('omit');
    // The page itself sends no Referer; Brandfetch hears which app searches, and no more.
    expect(init?.referrerPolicy).toBe('origin');
    expect(states.at(-1)).toMatchObject({ kind: 'done', query: 'example' });
  });

  it('aborts a search a newer one replaces and drops its answer', async () => {
    let release: (r: Response) => void = () => {};
    const first = new Promise<Response>((resolve) => (release = resolve));
    const { search, asked, states } = harness((url) => (url.pathname.endsWith('/slow') ? first : Response.json([])));
    search.type('slow');
    await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);
    search.type('fast');
    expect(asked[0].init?.signal?.aborted).toBe(true);
    await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);
    release(Response.json(SEARCH_HITS));
    await vi.advanceTimersByTimeAsync(0);
    expect(states.at(-1)).toEqual({ kind: 'done', query: 'fast', hits: [] });
    expect(states.some((s) => s.kind === 'done' && s.query === 'slow')).toBe(false);
  });

  it.each([
    [403, 'refused'],
    [401, 'refused'],
    [429, 'rate_limited'],
    [400, 'bad_query'],
    [503, 'unavailable'],
  ] as const)('maps HTTP %i to a %s message', async (status, problem) => {
    const { search, states } = harness(() => new Response(null, { status }));
    search.type('example');
    await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);
    expect(states.at(-1)).toEqual({ kind: 'failed', query: 'example', problem });
    expect(SEARCH_MESSAGES[problem]).toMatch(/\.$/);
  });

  it('reports a network failure as offline, and retry searches again at once', async () => {
    let down = true;
    const { search, asked, states } = harness(() => {
      if (down) throw new TypeError('Failed to fetch');
      return Response.json(SEARCH_HITS);
    });
    search.type('example');
    await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);
    expect(states.at(-1)).toEqual({ kind: 'failed', query: 'example', problem: 'offline' });
    down = false;
    search.retry();
    await vi.advanceTimersByTimeAsync(0);
    expect(asked).toHaveLength(2);
    expect(states.at(-1)).toMatchObject({ kind: 'done', query: 'example' });
  });

  it('forgets the hits when cleared or closed', async () => {
    const { search, states } = harness();
    search.type('example');
    await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);
    search.type('');
    expect(states.at(-1)).toEqual({ kind: 'idle' });
    search.type('ex');
    expect(states.at(-1)).toEqual({ kind: 'loading', query: 'ex', hits: [] });
    search.close();
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe('Brandfetch hits', () => {
  it('keep five well-formed hits and only Brandfetch-hosted HTTPS icons', () => {
    const hits = parseHits([
      { name: 'One', domain: 'one.example', brandId: 'id1', icon: 'https://cdn.brandfetch.io/id1/icon.png?c=test-client-id', claimed: true },
      { name: null, domain: 'two.example', brandId: 'id2', icon: 'https://evil.example/x.png' },
      { name: 'Three', domain: 'three.example', brandId: 'id3', icon: 'http://cdn.brandfetch.io/x.png' },
      { name: 'No domain', brandId: 'id4' },
      'junk',
      ...['a', 'b', 'c', 'd'].map((n) => ({ name: n, domain: `${n}.example`, brandId: `id-${n}`, icon: null })),
    ]);
    expect(hits).toHaveLength(5);
    expect(hits[0]).toEqual({ name: 'One', domain: 'one.example', brandId: 'id1', icon: 'https://cdn.brandfetch.io/id1/icon.png?c=test-client-id' });
    expect(hits[1]).toEqual({ name: null, domain: 'two.example', brandId: 'id2', icon: null });
    expect(hits[2].icon).toBeNull();
    expect(parseHits({ not: 'a list' })).toEqual([]);
  });

  it('encode the query into the path', () => {
    expect(searchUrl('test-client-id', 'a/b c')).toBe('https://api.brandfetch.io/v2/search/a%2Fb%20c?c=test-client-id&limit=5');
  });
});
