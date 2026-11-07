import type { AssetSource, BrandfetchStatus, ErrorBody } from '../api.ts';

// The Companion's Brandfetch settings and its Brand API pick, in memory. The key is write-only
// here too: nothing returns it. Picks answer from a recorded Brand API shape, never the network.

export interface FakeBrandfetchOptions {
  clientId?: string | null;
  keySet?: boolean;
  /** Provenance counts the Settings section shows. */
  logos?: { total: number; needRefresh: number; expired: string[] };
  /** False acts like a PC whose keychain can't be reached: the key can't be saved. */
  keychain?: boolean;
  /** Assets a refresh replaced, and their replacements. */
  replaced?: Record<string, string>;
}

// The Companion's own words, which name no platform: the UI words it for the platform.
const NO_KEYCHAIN = "The system keychain isn't available.";

/** The logo types the recorded Brand API answer has for every brand. */
const RECORDED_TYPES: ReadonlySet<string> = new Set(['icon', 'symbol', 'logo']);

const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
const failure = (status: number, code: string, message: string, details: unknown = null) => json(status, { error: { code, message, details } });
const CLIENT_ID = /^[A-Za-z0-9._-]{1,128}$/;

export function createFakeBrandfetch(options: FakeBrandfetchOptions = {}) {
  let clientId = options.clientId ?? null;
  let keySet = options.keySet ?? false;
  let total = options.logos?.total ?? 0;
  const needRefresh = options.logos?.needRefresh ?? 0;
  const expired = [...(options.logos?.expired ?? [])];
  const replaced: Record<string, string> = { ...options.replaced };
  const brands: string[] = [];
  let keyTests = 0;

  const status = (): BrandfetchStatus => ({ client_id: clientId, key_set: keySet, logos: total, need_refresh: needRefresh, expired: [...expired], replaced: { ...replaced } });

  async function body(request: Request): Promise<Record<string, unknown> | null> {
    try {
      const value: unknown = await request.json();
      return typeof value === 'object' && value !== null ? Object.fromEntries(Object.entries(value)) : null;
    } catch {
      return null;
    }
  }

  return {
    async handle(request: Request, path: string): Promise<Response | null> {
      const method = request.method;
      if (path === '/brandfetch') return method === 'GET' ? json(200, status()) : failure(405, 'route.method_not_allowed', 'Use GET.');
      if (path === '/brandfetch/client-id') {
        if (method === 'DELETE') {
          clientId = null;
          return json(200, status());
        }
        if (method !== 'PUT') return failure(405, 'route.method_not_allowed', 'Use PUT or DELETE.');
        const given = (await body(request))?.client_id;
        const id = typeof given === 'string' ? given.trim() : '';
        if (!CLIENT_ID.test(id)) return failure(400, 'request.invalid', 'The request is invalid.', { part: 'client_id', reason: 'format' });
        clientId = id;
        return json(200, status());
      }
      if (path === '/brandfetch/key/test') {
        if (method !== 'POST') return failure(405, 'route.method_not_allowed', 'Use POST.');
        if (!keySet) return failure(409, 'brandfetch.no_key', 'Add your Brandfetch API key in Settings, then retry.');
        keyTests += 1;
        return json(200, status());
      }
      if (path === '/brandfetch/key') {
        if (options.keychain === false) return failure(503, 'brandfetch.no_keychain', NO_KEYCHAIN);
        if (method === 'DELETE') {
          keySet = false;
          return json(200, status());
        }
        if (method !== 'PUT') return failure(405, 'route.method_not_allowed', 'Use PUT or DELETE.');
        const given = (await body(request))?.value;
        if (typeof given !== 'string' || !/^[\x21-\x7e]{1,512}$/.test(given.trim())) return failure(400, 'request.invalid', 'The request is invalid.', { part: 'value', reason: 'format' });
        keySet = true;
        return json(200, status());
      }
      return null;
    },
    /** A pick: the brand's recorded logo, or the error the Companion would give. */
    render(source: Extract<AssetSource, { kind: 'brandfetch' }>): { seed: string } | { error: ErrorBody } {
      if (!keySet) return { error: { code: 'brandfetch.no_key', message: 'Add your Brandfetch API key in Settings, then retry.', details: null } };
      if (source.brand.startsWith('missing')) return { error: { code: 'brandfetch.not_found', message: "Brandfetch doesn't know that brand.", details: null } };
      if (!RECORDED_TYPES.has(source.logo_type)) return { error: { code: 'brandfetch.no_logo', message: 'Brandfetch has no logo of that kind for this brand. Choose another result.', details: null } };
      brands.push(source.brand);
      total += 1;
      return { seed: `brandfetch:${source.brand}:${source.logo_type}` };
    },
    status,
    /** What a refresh does when a logo changed: `old` now leads to `next`. */
    replace(old: string, next: string) {
      for (const [from, to] of Object.entries(replaced)) if (to === old) replaced[from] = next;
      replaced[old] = next;
    },
    /** How many times Test key called the Brand API. */
    get keyTests(): number {
      return keyTests;
    },
    /** The brands picked so far, in order. */
    get picked(): readonly string[] {
      return brands;
    },
  };
}

export type FakeBrandfetch = ReturnType<typeof createFakeBrandfetch>;
