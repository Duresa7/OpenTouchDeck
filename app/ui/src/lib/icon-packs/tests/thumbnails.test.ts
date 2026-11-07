import { describe, expect, test } from 'vitest';
import { ApiError } from '../../companion/errors.ts';
import { createThumbnails, type ThumbnailDeps } from '../thumbnails.ts';

function harness(change: Partial<ThumbnailDeps> & { fail?: (name: string) => unknown } = {}) {
  const fetched: string[] = [];
  const revoked: string[] = [];
  let running = 0;
  let peak = 0;
  let minted = 0;
  const thumbnails = createThumbnails({
    fetch: async (prefix, name) => {
      fetched.push(`${prefix}/${name}`);
      running += 1;
      peak = Math.max(peak, running);
      await new Promise((resolve) => setTimeout(resolve, 1));
      running -= 1;
      const failure = change.fail?.(name);
      if (failure) throw failure;
      return new Blob(['<svg/>'], { type: 'text/plain' });
    },
    toUrl: (blob) => `blob:${(minted += 1)}:${blob.type}`,
    revoke: (url) => revoked.push(url),
    ...change,
  });
  return { thumbnails, fetched, revoked, peak: () => peak };
}

describe('pack thumbnails', () => {
  test('fetch each icon once, as SVG, and share requests in flight', async () => {
    const { thumbnails, fetched } = harness();
    expect(thumbnails.peek('mdi', 'spotify')).toBeUndefined();
    const [a, b] = await Promise.all([thumbnails.get('mdi', 'spotify'), thumbnails.get('mdi', 'spotify')]);
    expect(a).toEqual({ kind: 'url', url: 'blob:1:image/svg+xml' });
    expect(b).toBe(a);
    expect(thumbnails.peek('mdi', 'spotify')).toBe(a);
    await thumbnails.get('mdi', 'spotify');
    expect(fetched).toEqual(['mdi/spotify']);
  });

  test('remember a missing icon but retry a dropped request', async () => {
    const { thumbnails, fetched } = harness({
      fail: (name) => (name === 'gone' ? new ApiError('icon_pack.not_found', 'No such icon.') : name === 'flaky' ? new TypeError('offline') : null),
    });
    expect(await thumbnails.get('mdi', 'gone')).toEqual({ kind: 'missing' });
    expect(await thumbnails.get('mdi', 'gone')).toEqual({ kind: 'missing' });
    expect(await thumbnails.get('mdi', 'flaky')).toEqual({ kind: 'missing' });
    expect(thumbnails.peek('mdi', 'flaky')).toBeUndefined();
    await thumbnails.get('mdi', 'flaky');
    expect(fetched).toEqual(['mdi/gone', 'mdi/flaky', 'mdi/flaky']);
  });

  test('run a few requests at once and revoke what they drop', async () => {
    const { thumbnails, revoked, peak } = harness({ keep: 3, parallel: 2 });
    const urls = (await Promise.all(['a', 'b', 'c', 'd', 'e'].map((name) => thumbnails.get('mdi', name)))).map((t) => (t.kind === 'url' ? t.url : t.kind));
    expect(peak()).toBe(2);
    expect(revoked).toEqual(urls.slice(0, 2));
    await thumbnails.get('tabler', 'x');
    thumbnails.forget('mdi');
    // The oldest went as the cache filled, then the rest of mdi went with forget.
    expect(revoked).toEqual(urls);
    expect(thumbnails.peek('mdi', 'e')).toBeUndefined();
    expect(thumbnails.peek('tabler', 'x')).toBeDefined();
  });
});
