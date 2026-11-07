// Turning a Brandfetch hit into a Tile icon. The Companion calls the Brand API with the user's
// key and keeps the logo's provenance; the UI only names the brand.
import type { AssetRef, AssetSource, Platform } from '../companion/api.ts';
import { isApiError } from '../companion/errors.ts';
import { keychainMissing } from '../companion/keychain.ts';
import { BUSY, finish, UNREACHABLE, type PickDeps } from '../icons/pick.ts';
import type { BrandHit } from './search.ts';

/** What a Brandfetch pick is remembered as among recent icons: never the brand's name or URL. */
export const RECENT_NAME = 'Brandfetch logo';

export function brandfetchSource(hit: Pick<BrandHit, 'domain' | 'brandId'>): Extract<AssetSource, { kind: 'brandfetch' }> {
  return { kind: 'brandfetch', brand: hit.domain || hit.brandId, logo_type: 'icon', theme: 'auto', padding: 0, background: null, tint: null };
}

export async function renderBrandfetch(deps: PickDeps, hit: Pick<BrandHit, 'domain' | 'brandId'>, signal: AbortSignal): Promise<AssetRef> {
  const job = await deps.companion.assets.create({}, null, { source: brandfetchSource(hit), fit: 'contain' }, undefined, undefined, signal);
  return finish(deps, job, signal);
}

/** The copy guide's sentence for a failed pick. The picker shows the Companion's words under Details. */
export function pickProblem(error: unknown, name: string, platform: Platform | null): string {
  if (isApiError(error, 'client.unreachable')) return UNREACHABLE;
  if (isApiError(error, 'brandfetch.no_keychain')) return keychainMissing(platform);
  if (isApiError(error, 'brandfetch.no_key')) return 'Add your Brandfetch API key in Settings, Brandfetch logos, then retry.';
  if (isApiError(error, 'brandfetch.key_refused')) return 'Brandfetch refused the API key. Check it in Settings, Brandfetch logos.';
  if (isApiError(error, 'brandfetch.quota')) return 'Your Brandfetch API requests are used up. Check your plan in the Brandfetch developer portal.';
  if (isApiError(error, 'brandfetch.not_found')) return `Brandfetch has no details for ${name}. Choose another result.`;
  if (isApiError(error, 'brandfetch.no_logo')) return `Brandfetch has no logo for ${name} that this PC can download. Choose another result.`;
  if (isApiError(error, 'limits.exceeded')) return BUSY;
  return `Couldn't get ${name} from Brandfetch. Check the internet connection, then retry.`;
}
