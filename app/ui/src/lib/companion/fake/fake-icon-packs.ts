import type { CatalogHit, ErrorBody, Job, PackInfo, PackStage } from '../api.ts';

// The Icon pack routes of docs/companion-api.md: the curated list, install Jobs with their
// stages, remove, import, and SVG thumbnails. Pack icons are stand-ins seeded by name; the
// real ones come from the Companion.

type Frame = Record<string, unknown> & { type: string };

export interface FakeIconPacksOptions {
  publish(frame: Frame): void;
  /** Prefixes installed at the start. */
  installed?: readonly string[];
  /** How long each install stage takes. */
  stageMs?: number;
}

const json = (status: number, body: unknown) =>
  new Response(status === 204 ? null : JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

const failure = (status: number, code: string, message: string, details: unknown = null) => json(status, { error: { code, message, details } });

const IMPORT_MAX = 32 * 1024 * 1024;
/** The most imported packs, as the Companion keeps. */
const IMPORTED_MAX = 20;
const INK = '#f2f5fa';
const CUSTOM = 'Custom (you provided it)';
const NAME = /^[a-z0-9]+(-[a-z0-9]+)*$/;

const BRANDS = ['spotify', 'github', 'discord', 'slack', 'youtube', 'twitch', 'steam', 'chrome', 'firefox', 'figma', 'docker', 'whatsapp', 'telegram', 'notion', 'obs'];

type Curated = Omit<PackInfo, 'installed' | 'size_bytes' | 'origin'> & { names: readonly string[] };

const curated = (pack: Omit<Curated, 'names'>, names: readonly string[]): Curated => ({ ...pack, names: [...new Set([...names, ...pack.samples])].sort() });

export const FAKE_PACKS: readonly Curated[] = [
  curated(
    {
      prefix: 'logos', name: 'SVG Logos', version: '1.2.19', download_bytes: 8743040, icons: 2064,
      license: { title: 'CC0', spdx: 'CC0-1.0', url: 'https://raw.githubusercontent.com/gilbarbara/logos/master/LICENSE.txt' },
      author: { name: 'Gil Barbara', url: 'https://github.com/gilbarbara/logos' }, attribution: false, brands: true, palette: true,
      samples: ['npm-icon', 'uikit', 'patreon', 'serverless', 'vue', 'modernizr'],
    },
    [...BRANDS, ...BRANDS.map((b) => `${b}-icon`), 'visual-studio-code', 'microsoft-teams', 'netflix'],
  ),
  curated(
    {
      prefix: 'devicon', name: 'Devicon', version: '1.2.77', download_bytes: 5429359, icons: 1104,
      license: { title: 'MIT', spdx: 'MIT', url: 'https://github.com/devicons/devicon/blob/master/LICENSE' },
      author: { name: 'konpa', url: 'https://github.com/devicons/devicon/tree/master' }, attribution: false, brands: true, palette: true,
      samples: ['windows8', 'tensorflow', 'logstash', 'stackblitz', 'fsharp', 'vite'],
    },
    ['github', 'github-wordmark', 'docker', 'chrome', 'firefox', 'figma', 'slack', 'vscode', 'discordjs'],
  ),
  curated(
    {
      prefix: 'skill-icons', name: 'Skill Icons', version: '1.2.4', download_bytes: 1676441, icons: 400,
      license: { title: 'MIT', spdx: 'MIT', url: 'https://github.com/tandpfun/skill-icons/blob/main/LICENSE' },
      author: { name: 'tandpfun', url: 'https://github.com/tandpfun/skill-icons' }, attribution: false, brands: true, palette: true,
      samples: ['markdown-light', 'vuejs-dark', 'html', 'fastapi', 'vuejs-light', 'gamemakerstudio'],
    },
    ['spotify-dark', 'spotify-light', 'discord', 'github-dark', 'github-light', 'docker', 'figma-dark', 'figma-light'],
  ),
  curated(
    {
      prefix: 'fa6-brands', name: 'Font Awesome 6 Brands', version: '1.2.6', download_bytes: 510409, icons: 495,
      license: { title: 'CC BY 4.0', spdx: 'CC-BY-4.0', url: 'https://creativecommons.org/licenses/by/4.0/' },
      author: { name: 'Dave Gandy', url: 'https://github.com/FortAwesome/Font-Awesome' }, attribution: true, brands: true, palette: false,
      samples: ['strava', 'css3', 'y-combinator', 'unsplash', 'vuejs', 'google-drive'],
    },
    BRANDS,
  ),
  curated(
    {
      prefix: 'mdi', name: 'Material Design Icons', version: '1.2.3', download_bytes: 3472864, icons: 7447,
      license: { title: 'Apache 2.0', spdx: 'Apache-2.0', url: 'https://github.com/Templarian/MaterialDesign/blob/master/LICENSE' },
      author: { name: 'Pictogrammers', url: 'https://github.com/Templarian/MaterialDesign' }, attribution: false, brands: false, palette: false,
      samples: ['account-check', 'bell-alert-outline', 'calendar-edit', 'skip-previous', 'home-variant', 'lock-open-outline'],
    },
    ['spotify', 'github', 'discord', 'home', 'mail', 'music', 'play', 'pause', 'volume-high', 'cog', 'microphone', 'camera'],
  ),
  curated(
    {
      prefix: 'tabler', name: 'Tabler Icons', version: '1.2.43', download_bytes: 2148914, icons: 6257,
      license: { title: 'MIT', spdx: 'MIT', url: 'https://github.com/tabler/tabler-icons/blob/master/LICENSE' },
      author: { name: 'Paweł Kuna', url: 'https://github.com/tabler/tabler-icons' }, attribution: false, brands: false, palette: false,
      samples: ['alien', 'device-desktop', 'photo', 'chevron-right', 'check', 'square-root'],
    },
    ['brand-spotify', 'brand-spotify-filled', 'brand-github', 'brand-discord', 'home', 'mail', 'music', 'player-play', 'settings'],
  ),
  curated(
    {
      prefix: 'lucide', name: 'Lucide', version: '1.2.151', download_bytes: 664842, icons: 2020,
      license: { title: 'ISC', spdx: 'ISC', url: 'https://github.com/lucide-icons/lucide/blob/main/LICENSE' },
      author: { name: 'Lucide Contributors', url: 'https://github.com/lucide-icons/lucide' }, attribution: false, brands: false, palette: false,
      samples: ['circle-check', 'award', 'house', 'check', 'mountain', 'chevron-up'],
    },
    ['house', 'mail', 'music', 'play', 'pause', 'settings', 'github', 'mic', 'camera', 'volume-2'],
  ),
];

function hashOf(text: string): number {
  let h = 2166136261;
  for (const ch of text) h = Math.imul(h ^ (ch.codePointAt(0) ?? 0), 16777619);
  return h >>> 0;
}

const escape = (text: string) => text.replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);

/** A stand-in icon: a coloured badge for packs with their own colours, a line mark otherwise. */
export function fakeSvg(prefix: string, name: string, palette: boolean): string {
  const h = hashOf(`${prefix}:${name}`);
  const letter = escape((name.replace(/^brand-/, '').match(/[a-z0-9]/)?.[0] ?? '?').toUpperCase());
  const body = palette
    ? `<rect x="2" y="2" width="20" height="20" rx="6" fill="hsl(${h % 360} 62% 48%)"/><text x="12" y="16.2" font-family="system-ui,sans-serif" font-size="11" font-weight="700" text-anchor="middle" fill="#fff">${letter}</text>`
    : `<circle cx="12" cy="12" r="9" fill="none" stroke="currentColor" stroke-width="2"/><text x="12" y="16" font-family="system-ui,sans-serif" font-size="10" font-weight="600" text-anchor="middle" fill="currentColor">${letter}</text>`;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" color="${INK}">${body}</svg>`;
}

interface Imported {
  info: PackInfo;
  icons: Map<string, { body: string; width: number; height: number }>;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Validates an Iconify JSON file the way the Companion does, roughly. */
function parseImport(text: string): Imported | string {
  let data: unknown;
  try {
    data = JSON.parse(text);
  } catch {
    return 'not an Iconify JSON file';
  }
  if (!record(data) || typeof data.prefix !== 'string' || !NAME.test(data.prefix)) return 'the prefix must be lowercase letters, digits and single hyphens';
  if (!record(data.icons)) return 'not an Iconify JSON file';
  const width = typeof data.width === 'number' ? data.width : 16;
  const height = typeof data.height === 'number' ? data.height : 16;
  const icons = new Map<string, { body: string; width: number; height: number }>();
  for (const [name, icon] of Object.entries(data.icons)) {
    if (!NAME.test(name) || !record(icon) || typeof icon.body !== 'string' || /<script|href\s*=\s*["'](?!#)/i.test(icon.body)) continue;
    icons.set(name, { body: icon.body, width: typeof icon.width === 'number' ? icon.width : width, height: typeof icon.height === 'number' ? icon.height : height });
  }
  if (icons.size === 0) return 'the file holds no usable icons';
  const info = record(data.info) ? data.info : {};
  const author = record(info.author) && typeof info.author.name === 'string' ? info.author.name : '';
  return {
    icons,
    info: {
      prefix: data.prefix,
      name: typeof info.name === 'string' && info.name.trim() ? info.name.trim().slice(0, 80) : data.prefix,
      origin: 'imported',
      installed: true,
      version: null,
      download_bytes: null,
      size_bytes: text.length,
      icons: icons.size,
      license: { title: CUSTOM, spdx: null, url: null },
      author: { name: author.slice(0, 80), url: null },
      attribution: false,
      brands: false,
      palette: info.palette === true,
      samples: [...icons.keys()].slice(0, 6),
    },
  };
}

function hitOf(pack: PackInfo, name: string): CatalogHit {
  const license = pack.license.spdx ?? pack.license.title;
  let attribution = pack.attribution ? `Icons from ${pack.name} by ${pack.author.name} (${pack.license.title}); resized, padded or tinted.` : `Icons from ${pack.name} (${license}).`;
  if (pack.brands) attribution += ' Logos belong to their owners.';
  return {
    source: `iconify:${pack.prefix}`,
    name,
    id: name,
    aliases: [],
    hex: null,
    tags: [],
    category: pack.name,
    svg: true,
    light: false,
    dark: false,
    license,
    license_url: pack.license.url,
    guidelines: null,
    attribution_url: pack.author.url ?? pack.license.url ?? '',
    attribution,
  };
}

export function createFakeIconPacks(options: FakeIconPacksOptions) {
  const { publish, stageMs = 300 } = options;
  const installed = new Set(options.installed?.filter((prefix) => FAKE_PACKS.some((p) => p.prefix === prefix)));
  const imported = new Map<string, Imported>();
  const jobs = new Map<string, Job>();
  const failing = new Map<string, ErrorBody>();
  let minted = 0;

  function info(pack: Curated): PackInfo {
    const { names: _, ...rest } = pack;
    const here = installed.has(pack.prefix);
    return { ...rest, origin: 'curated', installed: here, size_bytes: here ? pack.download_bytes : null };
  }

  function list(): PackInfo[] {
    return [...FAKE_PACKS.map(info), ...[...imported.values()].map((p) => p.info).sort((a, b) => a.prefix.localeCompare(b.prefix))];
  }

  function update(job: Job, change: Partial<Job>): Job {
    const next = { ...job, ...change, rev: job.rev + 1 };
    jobs.set(next.id, next);
    publish({ type: 'job.updated', data: next });
    return next;
  }

  function install(pack: Curated): Job {
    minted += 1;
    const job: Job = {
      id: `job_pack${String(minted).padStart(12, '0')}`,
      kind: 'icon_packs.install',
      scope: { kind: 'companion' },
      state: 'running',
      progress: { stage: 'download' },
      result: null,
      error: null,
      cancellable: true,
      created_ms: Date.now(),
      finished_ms: null,
      rev: 1,
    };
    jobs.set(job.id, job);
    publish({ type: 'job.updated', data: job });
    const stages: PackStage[] = ['verify', 'extract'];
    const step = () => {
      const now = jobs.get(job.id);
      if (!now || now.state !== 'running') return;
      const stage = stages.shift();
      if (stage) {
        const refusal = stage === 'extract' ? failing.get(pack.prefix) : undefined;
        if (refusal) {
          failing.delete(pack.prefix);
          update(now, { state: 'failed', error: refusal, cancellable: false, finished_ms: Date.now() });
          return;
        }
        update(now, { progress: { stage } });
        setTimeout(step, stageMs);
        return;
      }
      installed.add(pack.prefix);
      update(now, { state: 'succeeded', progress: { stage: 'done' }, result: info(pack), cancellable: false, finished_ms: Date.now() });
    };
    setTimeout(step, stageMs);
    return job;
  }

  function packOf(prefix: string): { pack: PackInfo; names: readonly string[]; svg(name: string): string | null } | null {
    const curatedPack = FAKE_PACKS.find((p) => p.prefix === prefix);
    if (curatedPack && installed.has(prefix)) return { pack: info(curatedPack), names: curatedPack.names, svg: (name) => (curatedPack.names.includes(name) ? fakeSvg(prefix, name, curatedPack.palette) : null) };
    const mine = imported.get(prefix);
    if (!mine) return null;
    return {
      pack: mine.info,
      names: [...mine.icons.keys()],
      svg: (name) => {
        const icon = mine.icons.get(name);
        return icon ? `<svg xmlns="http://www.w3.org/2000/svg" width="${icon.width}" height="${icon.height}" viewBox="0 0 ${icon.width} ${icon.height}" color="${INK}">${icon.body}</svg>` : null;
      },
    };
  }

  return {
    async handle(request: Request, path: string): Promise<Response | null> {
      const method = request.method;
      const thumbnail = /^\/assets\/packs\/([^/]+)\/([^/]+)$/.exec(path);
      if (thumbnail) {
        if (method !== 'GET') return failure(405, 'route.method_not_allowed', 'Use GET.');
        const file = decodeURIComponent(thumbnail[2]);
        const svg = file.endsWith('.svg') ? packOf(decodeURIComponent(thumbnail[1]))?.svg(file.slice(0, -'.svg'.length)) : null;
        return svg ? new Response(svg, { status: 200, headers: { 'Content-Type': 'image/svg+xml' } }) : failure(404, 'icon_pack.not_found', 'No Icon pack or icon with that name.');
      }
      if (path === '/icon-packs') return method === 'GET' ? json(200, list()) : failure(405, 'route.method_not_allowed', 'Use GET.');
      if (path === '/icon-packs/import') {
        if (method !== 'POST') return failure(405, 'route.method_not_allowed', 'Use POST.');
        if ((request.headers.get('Content-Type') ?? '').split(';')[0].trim() !== 'application/json') return failure(415, 'request.media_type', 'The request requires JSON.');
        const text = await request.text();
        if (text.length > IMPORT_MAX) return failure(413, 'request.too_large', 'The request body is too large.');
        const parsed = parseImport(text);
        if (typeof parsed === 'string') return failure(422, 'icon_pack.invalid', `This isn't a usable Iconify JSON pack: ${parsed}.`);
        const prefix = parsed.info.prefix;
        if (FAKE_PACKS.some((p) => p.prefix === prefix) || imported.has(prefix)) return failure(409, 'icon_pack.exists', `An Icon pack named "${prefix}" is already installed or reserved.`);
        if (imported.size >= IMPORTED_MAX) return failure(409, 'icon_pack.too_many', `${IMPORTED_MAX} imported Icon packs are installed, the most the Companion keeps.`);
        imported.set(prefix, parsed);
        return json(200, parsed.info);
      }
      const one = /^\/icon-packs\/([^/]+)$/.exec(path);
      if (one) {
        const prefix = decodeURIComponent(one[1]);
        if (method === 'POST') {
          const pack = FAKE_PACKS.find((p) => p.prefix === prefix);
          if (!pack) return failure(404, 'icon_pack.not_found', 'No Icon pack or icon with that name.');
          return json(202, install(pack));
        }
        if (method === 'DELETE') {
          if (installed.delete(prefix) || imported.delete(prefix)) return json(204, null);
          return failure(404, 'icon_pack.not_found', 'No Icon pack or icon with that name.');
        }
        return failure(405, 'route.method_not_allowed', 'Use POST or DELETE.');
      }
      const job = /^\/jobs\/(job_pack[^/]+)$/.exec(path);
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
    /** Search hits for every installed pack, in prefix then name order, for the catalog search. */
    hits(): CatalogHit[] {
      return list()
        .filter((p) => p.installed)
        .sort((a, b) => a.prefix.localeCompare(b.prefix))
        .flatMap((p) => (packOf(p.prefix)?.names ?? []).map((name) => hitOf(p, name)));
    },
    jobs: () => [...jobs.values()],
    installed: () => list().filter((p) => p.installed).map((p) => p.prefix),
    /** The next install of `prefix` fails with this error while it unpacks. */
    failInstall(prefix: string, error: ErrorBody) {
      failing.set(prefix, error);
    },
  };
}

export type FakeIconPacks = ReturnType<typeof createFakeIconPacks>;
