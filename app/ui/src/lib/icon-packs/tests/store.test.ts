import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import { createClient, type Companion } from '../../companion/client.ts';
import { createFakeCompanion, FAKE_PACKS, type FakeCompanion } from '../../companion/fake.ts';
import { createJobsStore, waitForJob } from '../../companion/jobs.ts';
import { windowSession } from '../../companion/session.ts';
import { createIconPacks, type IconPacks } from '../store.ts';

const BASE = 'http://127.0.0.1:51234';

let fake: FakeCompanion;
let companion: Companion;
let packs: IconPacks;
let changed: string[];

const state = () => packs.store.getState();
const info = (prefix: string) => state().packs?.find((p) => p.prefix === prefix);
const search = (q: string) => companion.assets.catalog_search({}, { q, catalog: null });

const pack = (prefix: string, icons: Record<string, unknown>) =>
  new Blob([JSON.stringify({ prefix, info: { name: 'My icons', author: { name: 'Me' } }, icons })], { type: 'application/octet-stream' });

beforeEach(async () => {
  fake = createFakeCompanion({ iconPacks: { installed: ['mdi'], stageMs: 1 } });
  vi.stubGlobal('fetch', fake.fetch);
  companion = createClient(BASE, windowSession(fake.bridge));
  const jobs = createJobsStore();
  changed = [];
  packs = createIconPacks({
    companion,
    wait: (job) => waitForJob<'icon_packs.install'>(jobs, companion, job, { pollMs: 2 }),
    changed: (prefix) => changed.push(prefix),
  });
  await packs.load();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('the pack list', () => {
  test('offers the curated packs with nothing installed but what the Companion has', () => {
    expect(state().load).toBe('ready');
    expect(state().packs?.map((p) => p.prefix)).toEqual(FAKE_PACKS.map((p) => p.prefix));
    expect(state().packs?.map((p) => p.prefix).sort()).toEqual(['devicon', 'fa6-brands', 'logos', 'lucide', 'mdi', 'skill-icons', 'tabler']);
    expect(state().packs?.filter((p) => p.installed).map((p) => p.prefix)).toEqual(['mdi']);
  });

  test('reports a list that will not load, once, until a retry works', async () => {
    const fresh = createIconPacks({ companion, wait: () => Promise.reject(new Error('unused')) });
    vi.stubGlobal('fetch', () => Promise.reject(new TypeError('offline')));
    await Promise.all([fresh.load(), fresh.load()]);
    expect(fresh.store.getState()).toMatchObject({ load: 'failed', packs: null });
    vi.stubGlobal('fetch', fake.fetch);
    await fresh.load();
    expect(fresh.store.getState()).toMatchObject({ load: 'ready', loadProblem: null });
  });
});

describe('installing a pack', () => {
  test('runs the install Job, then its icons join the catalog search', async () => {
    expect((await search('spotify')).some((h) => h.source === 'iconify:logos')).toBe(false);
    const seen: (string | undefined)[] = [];
    const stop = packs.store.subscribe((s) => seen.push(s.work.logos?.kind));
    await packs.install('logos');
    stop();
    expect(seen).toContain('installing');
    expect(state().work.logos).toBeUndefined();
    expect(info('logos')).toMatchObject({ installed: true, size_bytes: expect.any(Number) });
    expect(changed).toEqual(['logos']);
    const hits = await search('spotify');
    expect(hits.filter((h) => h.source.startsWith('iconify:')).map((h) => h.source)).toEqual(expect.arrayContaining(['iconify:logos', 'iconify:mdi']));
  });

  test('keeps the pack offered, with the reason, when the Job fails', async () => {
    fake.iconPacks.failInstall('lucide', { code: 'icon_pack.integrity', message: 'The download did not match its pinned checksum.', details: null });
    await packs.install('lucide');
    expect(info('lucide')?.installed).toBe(false);
    expect(state().work.lucide).toEqual({ kind: 'failed', text: "Lucide didn't match its checksum, so it wasn't installed. Retry later.", detail: 'The download did not match its pinned checksum.' });
    expect(changed).toEqual([]);
    await packs.install('lucide');
    expect(info('lucide')?.installed).toBe(true);
    expect(state().work.lucide).toBeUndefined();
  });
});

describe('removing a pack', () => {
  test('deletes it and drops its icons from search', async () => {
    await packs.remove('mdi');
    expect(info('mdi')).toMatchObject({ installed: false, size_bytes: null });
    expect(changed).toEqual(['mdi']);
    expect((await search('spotify')).some((h) => h.source === 'iconify:mdi')).toBe(false);
  });
});

describe('importing a pack file', () => {
  test('adds a custom pack whatever type the file picker gave it', async () => {
    await packs.importFile(pack('mine', { spotify: { body: '<circle cx="8" cy="8" r="8" fill="currentColor"/>' } }), 'mine.json');
    expect(state()).toMatchObject({ importing: null, importProblem: null });
    expect(info('mine')).toMatchObject({ origin: 'imported', installed: true, license: { title: 'Custom (you provided it)' } });
    expect(changed).toEqual(['mine']);
    expect((await search('spotify')).some((h) => h.source === 'iconify:mine')).toBe(true);
  });

  test('refuses a prefix clash and a file that is not Iconify JSON', async () => {
    await packs.importFile(pack('logos', { a: { body: '<path d="M0 0h1v1z"/>' } }), 'logos.json');
    expect(state().importProblem?.text).toBe('A pack with the same prefix as “logos.json” is already installed or offered above.');
    await packs.importFile(new Blob(['<svg/>']), 'icon.svg');
    expect(state().importProblem?.text).toBe("“icon.svg” isn't an Iconify JSON pack. Choose an Iconify JSON file, such as icons.json.");
    expect(state().packs?.some((p) => p.origin === 'imported')).toBe(false);
  });
});
