import { encodeTdi1, TDI1_SIDE } from '../../icons/tdi1.ts';
import type { AssetRef, AssetSource, Builtin, CatalogHit, CreateAsset, ErrorBody, Fit, Job } from '../api.ts';
import type { FakeBrandfetch } from './fake-brandfetch.ts';
import { RECORDED_BUILTINS, RECORDED_HITS } from './fake-catalog.data.ts';
import { fakeGif } from './fake-gif.ts';

type Frame = Record<string, unknown> & { type: string };

export interface FakeAssetsOptions {
  publish(frame: Frame): void;
  hits?: readonly CatalogHit[];
  /** Hits from installed Icon packs, searched after the catalog within a rank. */
  packHits?: () => readonly CatalogHit[];
  renderMs?: number;
  searchMs?: number;
  /** Answers Brandfetch picks; without it they fail as with no API key. */
  brandfetch?: Pick<FakeBrandfetch, 'render'>;
}

export interface FakeSearch {
  q: string;
  catalog: string | null;
  aborted: boolean;
}

const json = (status: number, body: unknown) =>
  new Response(status === 204 ? null : JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

const failure = (status: number, code: string, message: string, details: unknown = null) => json(status, { error: { code, message, details } });

const UPLOAD_MAX = 16 * 1024 * 1024;
const GIF_SIDE = 128;
const UPLOAD_TYPES: ReadonlySet<string> = new Set(['image/png', 'image/jpeg', 'image/gif', 'image/svg+xml']);
const SEARCH_LIMIT = 500;

function pause(ms: number | Promise<void>, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) return reject(new DOMException('The request was aborted.', 'AbortError'));
    const go = () => {
      signal.removeEventListener('abort', stop);
      resolve();
    };
    const timer = typeof ms === 'number' ? setTimeout(go, ms) : (void ms.then(go), null);
    const stop = () => {
      if (timer !== null) clearTimeout(timer);
      reject(new DOMException('The request was aborted.', 'AbortError'));
    };
    signal.addEventListener('abort', stop, { once: true });
  });
}

async function sha256(bytes: Uint8Array): Promise<string> {
  const copy = new Uint8Array(bytes);
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', copy.buffer));
  return [...digest].map((b) => b.toString(16).padStart(2, '0')).join('');
}

function hashOf(text: string): number {
  let h = 2166136261;
  for (const ch of text) h = Math.imul(h ^ (ch.codePointAt(0) ?? 0), 16777619);
  return h >>> 0;
}

function disc(seed: string, tint: readonly [number, number, number] | null): Uint8Array {
  const h = hashOf(seed);
  const color = tint ?? [96 + (h & 0x7f), 96 + ((h >> 8) & 0x7f), 96 + ((h >> 16) & 0x7f)];
  const ring = 6 + (h % 10);
  const centre = (TDI1_SIDE - 1) / 2;
  return encodeTdi1((x, y) => {
    const d = Math.hypot(x - centre, y - centre);
    const on = d <= 28 && (d >= 28 - ring || d <= 8);
    return [color[0], color[1], color[2], on ? 255 : 0];
  });
}

function rank(hit: CatalogHit, query: string): number | null {
  let best: number | null = null;
  for (const word of [hit.name, ...hit.aliases, ...hit.tags]) {
    const text = word.toLowerCase();
    const r = text === query ? 0 : text.startsWith(query) ? 1 : text.includes(query) ? 2 : null;
    if (r !== null && (best === null || r < best)) best = r;
  }
  return best;
}

export function searchHits(hits: readonly CatalogHit[], q: string, catalog: string | null): CatalogHit[] {
  const query = q.trim().toLowerCase();
  return hits
    .filter((hit) => catalog === null || hit.source === catalog)
    .flatMap((hit, index) => {
      const r = rank(hit, query);
      return r === null ? [] : [{ r, index, hit }];
    })
    .sort((a, b) => a.r - b.r || a.index - b.index)
    .map(({ hit }) => hit);
}

export function createFakeAssets(options: FakeAssetsOptions) {
  const { publish, renderMs = 250, searchMs = 30 } = options;
  let hits = options.hits ?? RECORDED_HITS;
  const everything = () => [...hits, ...(options.packHits?.() ?? [])];
  const files = new Map<string, Uint8Array>();
  const uploads = new Map<string, Uint8Array>();
  const jobs = new Map<string, Job>();
  const created: AssetSource[] = [];
  const fits: Fit[] = [];
  const gifFailures: ErrorBody[] = [];
  const waitingGifs: (() => void)[] = [];
  let gifsHeld = false;
  const searches: FakeSearch[] = [];
  const failing = new Set<string>();
  let offline = false;
  let held: { promise: Promise<void>; release(): void } | null = null;
  let minted = 0;
  let builtins: Promise<Builtin[]> | null = null;

  function listBuiltins(): Promise<Builtin[]> {
    builtins ??= Promise.all(
      RECORDED_BUILTINS.map(async (icon) => {
        const file = disc(`builtin:${icon.name}`, [0xf0, 0xf0, 0xf8]);
        const id = (await sha256(file)).slice(0, 16);
        files.set(id, file);
        return { ...icon, tags: [...icon.tags], id };
      }),
    );
    return builtins;
  }

  function update(job: Job, change: Partial<Job>): Job {
    const next = { ...job, ...change, rev: job.rev + 1 };
    jobs.set(next.id, next);
    publish({ type: 'job.updated', data: next });
    return next;
  }

  async function render(source: AssetSource, fit: Fit): Promise<{ file: Uint8Array } | { problem: string } | { error: ErrorBody }> {
    switch (source.kind) {
      case 'catalog': {
        const hit = everything().find((h) => h.source === source.catalog && h.id === source.id);
        if (!hit) return { problem: 'Catalog icon or variant not found' };
        if (failing.has(source.id)) return { problem: 'Icon download failed: the CDN did not answer' };
        return { file: disc(`catalog:${source.catalog}:${source.id}`, source.tint) };
      }
      case 'gif': {
        const bytes = uploads.get(source.upload);
        uploads.delete(source.upload);
        if (!bytes) return { problem: 'upload.not_found' };
        const { width, height } = source.target;
        const scale = Math.min(1, GIF_SIDE / width, GIF_SIDE / height);
        const seed = hashOf(`${await sha256(bytes)}:${width}x${height}:${fit}`);
        return { file: fakeGif({ seed, width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) }) };
      }
      case 'upload': {
        const bytes = uploads.get(source.upload);
        uploads.delete(source.upload);
        if (!bytes) return { problem: 'upload.not_found' };
        return { file: disc(`upload:${await sha256(bytes)}`, null) };
      }
      case 'app':
        return source.path ? { file: disc(`app:${source.path.toLowerCase()}`, null) } : { problem: 'no icon resource in that program' };
      case 'brandfetch': {
        const picked = options.brandfetch?.render(source) ?? { error: { code: 'brandfetch.no_key', message: 'Add your Brandfetch API key in Settings, then retry.', details: null } };
        return 'error' in picked ? picked : { file: disc(picked.seed, source.tint) };
      }
      default: {
        const exhaustive: never = source;
        return exhaustive;
      }
    }
  }

  async function finish(id: string, body: CreateAsset, refusal: ErrorBody | undefined) {
    const outcome = await render(body.source, body.fit ?? 'contain');
    const now = jobs.get(id);
    if (!now || now.state !== 'running') return;
    if (refusal) {
      update(now, { state: 'failed', error: refusal, cancellable: false, finished_ms: Date.now() });
      return;
    }
    if ('error' in outcome) {
      update(now, { state: 'failed', error: outcome.error, cancellable: false, finished_ms: Date.now() });
      return;
    }
    if ('problem' in outcome) {
      const code = outcome.problem === 'upload.not_found' ? 'upload.not_found' : 'asset.render_failed';
      const message = code === 'upload.not_found' ? 'The upload expired or never existed.' : `The image or icon couldn't be rendered: ${outcome.problem}`;
      update(now, { state: 'failed', error: { code, message, details: null }, cancellable: false, finished_ms: Date.now() });
      return;
    }
    const fileId = await sha256(outcome.file);
    files.set(fileId, outcome.file);
    const gif = body.source.kind === 'gif';
    const size = gif ? { width: outcome.file[6] | (outcome.file[7] << 8), height: outcome.file[8] | (outcome.file[9] << 8) } : { width: TDI1_SIDE, height: TDI1_SIDE };
    const result: AssetRef = { id: fileId, kind: gif ? 'gif' : 'tdi1', ...size, bytes: outcome.file.length };
    update(now, { state: 'succeeded', progress: { frames_done: 1, frames_total: 1 }, result, cancellable: false, finished_ms: Date.now() });
  }

  function startRender(body: CreateAsset): Job {
    minted += 1;
    created.push(body.source);
    fits.push(body.fit ?? 'contain');
    const gif = body.source.kind === 'gif';
    const job: Job = {
      id: `job_asset${String(minted).padStart(11, '0')}`,
      kind: 'assets.render',
      scope: { kind: 'companion' },
      state: gif && gifsHeld ? 'queued' : 'running',
      progress: { frames_done: 0, frames_total: 1 },
      result: null,
      error: null,
      cancellable: true,
      created_ms: Date.now(),
      finished_ms: null,
      rev: 1,
    };
    jobs.set(job.id, job);
    publish({ type: 'job.updated', data: job });
    const refusal = gif ? gifFailures.shift() : undefined;
    const run = () => setTimeout(() => void finish(job.id, body, refusal), renderMs);
    if (job.state === 'running') {
      run();
      return job;
    }
    waitingGifs.push(() => {
      const now = jobs.get(job.id);
      if (now?.state !== 'queued') return;
      update(now, { state: 'running' });
      run();
    });
    return job;
  }

  function isCreateAsset(value: unknown): value is CreateAsset {
    if (typeof value !== 'object' || value === null || !('source' in value)) return false;
    if ('fit' in value && value.fit !== undefined && value.fit !== null && value.fit !== 'contain' && value.fit !== 'cover') return false;
    const source = value.source;
    if (typeof source === 'object' && source !== null && 'kind' in source && source.kind === 'gif') {
      if (!('upload' in source) || typeof source.upload !== 'string' || !('device' in source) || typeof source.device !== 'string' || !('target' in source)) return false;
      const target = source.target;
      return typeof target === 'object' && target !== null && 'width' in target && typeof target.width === 'number' && Number.isInteger(target.width) && target.width > 0 && target.width <= 4096 && 'height' in target && typeof target.height === 'number' && Number.isInteger(target.height) && target.height > 0 && target.height <= 4096 && 'background' in target && Array.isArray(target.background) && target.background.length === 3 && target.background.every((v: unknown) => typeof v === 'number' && Number.isInteger(v) && v >= 0 && v <= 255);
    }
    return typeof source === 'object'  && source !== null && 'kind' in source && ['app', 'brandfetch', 'catalog', 'upload'].includes(String(source.kind));
  }

  return {
    async handle(request: Request, path: string): Promise<Response | null> {
      const method = request.method;
      const assetRoute = path === '/uploads' || path.startsWith('/assets');
      if (!assetRoute && !/^\/jobs\/job_asset/.test(path)) return null;
      if (offline && assetRoute) throw new TypeError('Failed to fetch');
      if (path === '/assets/catalog/search') {
        if (method !== 'GET') return failure(405, 'route.method_not_allowed', 'Use GET.');
        const url = new URL(request.url);
        const entry: FakeSearch = { q: url.searchParams.get('q') ?? '', catalog: url.searchParams.get('catalog'), aborted: false };
        searches.push(entry);
        try {
          await pause(held?.promise ?? searchMs, request.signal);
        } catch (error) {
          entry.aborted = true;
          throw error;
        }
        return json(200, searchHits(everything(), entry.q, entry.catalog).slice(0, SEARCH_LIMIT));
      }
      if (path === '/assets/builtin') return method === 'GET' ? json(200, await listBuiltins()) : failure(405, 'route.method_not_allowed', 'Use GET.');
      if (path === '/assets/builtin/suggest') {
        if (method !== 'POST') return failure(405, 'route.method_not_allowed', 'Use POST.');
        const body: unknown = await request.json();
        const type = typeof body === 'object' && body !== null && 'action' in body && typeof body.action === 'object' && body.action !== null && 'type' in body.action ? body.action.type : null;
        const name = type === 'consumer' ? 'music' : type === 'launch' ? 'browser' : type === 'command' ? 'terminal' : type === 'open' ? 'folder' : null;
        return json(200, (await listBuiltins()).find((icon) => icon.name === name) ?? null);
      }
      if (path === '/uploads') {
        if (method !== 'POST') return failure(405, 'route.method_not_allowed', 'Use POST.');
        const type = request.headers.get('Content-Type') ?? '';
        if (!UPLOAD_TYPES.has(type)) return failure(415, 'request.media_type', 'Uploads take PNG, JPEG, GIF or SVG images.');
        const bytes = new Uint8Array(await request.arrayBuffer());
        if (bytes.length > UPLOAD_MAX) return failure(413, 'request.too_large', 'The image is larger than 16 MiB.');
        const id = `upl_${String(uploads.size + 1).padStart(16, '0')}`;
        uploads.set(id, bytes);
        return json(200, { id, media_type: type, bytes: bytes.length });
      }
      if (path === '/assets') {
        if (method !== 'POST') return failure(405, 'route.method_not_allowed', 'Use POST.');
        const body: unknown = await request.json();
        if (!isCreateAsset(body)) return failure(400, 'request.invalid', 'Expected a source.', { part: 'body', reason: 'source' });
        return json(202, startRender(body));
      }
      const file = /^\/assets\/([^/]+)$/.exec(path);
      if (file) {
        if (method !== 'GET') return failure(405, 'route.method_not_allowed', 'Use GET.');
        await listBuiltins();
        const bytes = files.get(decodeURIComponent(file[1]));
        if (!bytes) return failure(404, 'asset.not_found', 'No Asset with that id.');
        return new Response(new Uint8Array(bytes), { status: 200, headers: { 'Content-Type': bytes[0] === 71 ? 'image/gif' : 'application/vnd.touchdeck.tdi1' } });
      }
      const job = /^\/jobs\/([^/]+)$/.exec(path);
      if (job) {
        const found = jobs.get(decodeURIComponent(job[1]));
        if (!found) return failure(404, 'job.not_found', 'No Job with that id.');
        if (method === 'GET') return json(200, found);
        if (method !== 'DELETE') return failure(405, 'route.method_not_allowed', 'Use GET or DELETE.');
        if (!found.cancellable || found.finished_ms !== null) return failure(409, 'job.not_cancellable', "The Job's current phase can't be cancelled.");
        return json(200, update(found, { state: 'cancelled', error: { code: 'job.cancelled', message: 'The Job was cancelled.', details: null }, cancellable: false, finished_ms: Date.now() }));
      }
      return null;
    },
    jobs: () => [...jobs.values()],
    get created(): readonly AssetSource[] {
      return created;
    },
    /** The fit each `assets.create` asked for, in order, `contain` when it named none. */
    get fits(): readonly Fit[] {
      return fits;
    },
    /** While on, a new GIF Job waits as `queued`, the way the Companion queues a second GIF. */
    holdGifs(on: boolean) {
      gifsHeld = on;
      if (!on) for (const go of waitingGifs.splice(0)) go();
    },
    /** The next GIF Job fails with this error. */
    failGif(error: ErrorBody) {
      gifFailures.push(error);
    },
    get searches(): readonly FakeSearch[] {
      return searches;
    },
    setOffline(on: boolean) {
      offline = on;
    },
    holdSearches(on: boolean) {
      if (on && !held) {
        let release = () => {};
        const promise = new Promise<void>((resolve) => (release = resolve));
        held = { promise, release };
      }
      if (!on && held) {
        held.release();
        held = null;
      }
    },
    failRender(id: string) {
      failing.add(id);
    },
    useHits(next: readonly CatalogHit[]) {
      hits = next;
    },
  };
}

export type FakeAssets = ReturnType<typeof createFakeAssets>;
