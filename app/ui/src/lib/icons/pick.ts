import type { AssetRef, CatalogHit, Job } from '../companion/api.ts';
import type { Companion } from '../companion/client.ts';
import { isApiError } from '../companion/errors.ts';
import { catalogSource } from './catalog.ts';

export interface PickDeps {
  companion: Pick<Companion, 'assets' | 'jobs'>;
  wait(job: Job, signal: AbortSignal): Promise<AssetRef>;
}

export const UNREACHABLE = "Can't reach the Companion. Check that Open TouchDeck is running in the tray, then retry.";
export const ICONS_FAILED = "Icons didn't load. Restart Open TouchDeck, then retry.";
export const BUSY = 'Other icons are still being prepared. Wait a moment, then retry.';

/** Waits for a render Job, and cancels it when the pick is abandoned. */
export async function finish(deps: PickDeps, job: Job, signal: AbortSignal): Promise<AssetRef> {
  try {
    return await deps.wait(job, signal);
  } catch (error) {
    if (signal.aborted) deps.companion.jobs.cancel({ job: job.id }).catch(() => {});
    throw error;
  }
}

export async function renderCatalog(deps: PickDeps, hit: Pick<CatalogHit, 'source' | 'id' | 'hex'>, signal: AbortSignal): Promise<AssetRef> {
  const job = await deps.companion.assets.create({}, null, { source: catalogSource(hit), fit: 'contain' }, undefined, undefined, signal);
  return finish(deps, job, signal);
}

export async function renderUpload(deps: PickDeps, file: Blob, signal: AbortSignal): Promise<AssetRef> {
  const uploaded = await deps.companion.assets.upload({}, null, file, undefined, undefined, signal);
  const job = await deps.companion.assets.create({}, null, { source: { kind: 'upload', upload: uploaded.id }, fit: 'contain' }, undefined, undefined, signal);
  return finish(deps, job, signal);
}

export async function renderApp(deps: PickDeps, path: string, signal: AbortSignal): Promise<AssetRef> {
  const job = await deps.companion.assets.create({}, null, { source: { kind: 'app', path }, fit: 'contain' }, undefined, undefined, signal);
  return finish(deps, job, signal);
}

export function catalogProblem(error: unknown, name: string): string {
  if (isApiError(error, 'client.unreachable')) return UNREACHABLE;
  if (isApiError(error, 'limits.exceeded')) return BUSY;
  return `Couldn't get ${name}. Check the internet connection, then retry.`;
}

export function uploadProblem(error: unknown, file: string): string {
  if (isApiError(error, 'client.unreachable')) return UNREACHABLE;
  if (isApiError(error, 'limits.exceeded')) return BUSY;
  return `Couldn't upload “${file}”. Choose a PNG, JPEG or SVG image up to 16 MB.`;
}

export function appProblem(error: unknown): string {
  if (isApiError(error, 'client.unreachable')) return UNREACHABLE;
  if (isApiError(error, 'limits.exceeded')) return BUSY;
  return "Couldn't get that app's icon. Check the app's path, or choose an icon.";
}

export function searchProblem(error: unknown): string {
  return isApiError(error, 'client.unreachable') ? UNREACHABLE : ICONS_FAILED;
}
