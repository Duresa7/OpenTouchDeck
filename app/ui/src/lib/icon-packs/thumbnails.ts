import { isApiError } from '../companion/errors.ts';

/** A pack icon's thumbnail: an object URL for its SVG, or missing once the Companion says so. */
export type Thumbnail = { kind: 'url'; url: string } | { kind: 'missing' };

export interface ThumbnailDeps {
  /** Fetches `<name>.svg` from `GET /assets/packs/{prefix}/{icon}`. */
  fetch(prefix: string, name: string): Promise<Blob>;
  toUrl(blob: Blob): string;
  revoke(url: string): void;
  /** How many thumbnails stay cached. The oldest URLs are revoked past it. */
  keep?: number;
  /** How many requests run at once. */
  parallel?: number;
}

export interface Thumbnails {
  /** The cached thumbnail, without fetching. */
  peek(prefix: string, name: string): Thumbnail | undefined;
  get(prefix: string, name: string): Promise<Thumbnail>;
  /** Drops a pack's thumbnails, after it is removed or replaced. */
  forget(prefix: string): void;
}

const keyOf = (prefix: string, name: string) => `${prefix}/${name}`;

export function createThumbnails({ fetch, toUrl, revoke, keep = 400, parallel = 6 }: ThumbnailDeps): Thumbnails {
  const done = new Map<string, Thumbnail>();
  const pending = new Map<string, Promise<Thumbnail>>();
  const queue: (() => void)[] = [];
  let running = 0;

  const release = (thumbnail: Thumbnail) => {
    if (thumbnail.kind === 'url') revoke(thumbnail.url);
  };

  const store = (key: string, value: Thumbnail) => {
    done.delete(key);
    done.set(key, value);
    for (const [oldest, dropped] of done) {
      if (done.size <= keep) break;
      done.delete(oldest);
      release(dropped);
    }
  };

  const slot = () =>
    new Promise<void>((resolve) => {
      if (running < parallel) {
        running += 1;
        resolve();
      } else {
        queue.push(() => {
          running += 1;
          resolve();
        });
      }
    });

  const free = () => {
    running -= 1;
    queue.shift()?.();
  };

  return {
    peek(prefix, name) {
      const key = keyOf(prefix, name);
      const value = done.get(key);
      if (value !== undefined) store(key, value);
      return value;
    },
    get(prefix, name) {
      const key = keyOf(prefix, name);
      const known = done.get(key);
      if (known !== undefined) return Promise.resolve(known);
      let loading = pending.get(key);
      if (!loading) {
        let lasting = true;
        loading = (async (): Promise<Thumbnail> => {
          await slot();
          try {
            const blob = await fetch(prefix, name);
            return { kind: 'url', url: toUrl(blob.type === 'image/svg+xml' ? blob : new Blob([blob], { type: 'image/svg+xml' })) };
          } catch (error) {
            // Only the Companion's own answer is final; a dropped request may work next time.
            lasting = isApiError(error, 'icon_pack.not_found');
            return { kind: 'missing' };
          } finally {
            free();
          }
        })().then((value) => {
          if (pending.get(key) === loading) {
            pending.delete(key);
            if (lasting) store(key, value);
          } else {
            release(value);
          }
          return value;
        });
        pending.set(key, loading);
      }
      return loading;
    },
    forget(prefix) {
      for (const [key, value] of [...done]) {
        if (!key.startsWith(`${prefix}/`)) continue;
        done.delete(key);
        release(value);
      }
      for (const key of [...pending.keys()]) if (key.startsWith(`${prefix}/`)) pending.delete(key);
    },
  };
}
