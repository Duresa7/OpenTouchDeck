import type { CatalogHit, PackInfo, PackStage } from '../companion/api.ts';
import { isApiError } from '../companion/errors.ts';

/** The catalog name a pack hit carries, followed by the pack's prefix. */
export const PACK_SOURCE = 'iconify:';

export const isPackHit = (hit: Pick<CatalogHit, 'source'>): boolean => hit.source.startsWith(PACK_SOURCE);

export function packPrefixOf(source: string): string | null {
  return source.startsWith(PACK_SOURCE) ? source.slice(PACK_SOURCE.length) || null : null;
}

/** The label a picker cell shows for a hit's source: a catalog's name, or the pack's. */
export function sourceName(hit: Pick<CatalogHit, 'source' | 'category'>, catalogs: Readonly<Record<string, string>>): string {
  if (isPackHit(hit)) return hit.category ?? packPrefixOf(hit.source) ?? hit.source;
  return catalogs[hit.source] ?? hit.source;
}

/** Bytes in KB and MB of 1024. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  const mb = bytes / (1024 * 1024);
  return `${mb < 10 ? mb.toFixed(1) : Math.round(mb)} MB`;
}

const count = new Intl.NumberFormat('en-US');

/** "2,064 icons · 8.3 MB": the size on disk once installed, the download size before. */
export function packFacts(pack: Pick<PackInfo, 'icons' | 'installed' | 'size_bytes' | 'download_bytes'>): string {
  const icons = `${count.format(pack.icons)} ${pack.icons === 1 ? 'icon' : 'icons'}`;
  const bytes = pack.installed ? pack.size_bytes : pack.download_bytes;
  return bytes === null ? icons : `${icons} · ${formatBytes(bytes)}`;
}

export interface PackCredit {
  prefix: string;
  name: string;
  author: string;
  license: string;
  url: string | null;
}

/** The installed packs whose license asks for credit wherever their icons show. */
export function credits(packs: readonly PackInfo[]): PackCredit[] {
  return packs
    .filter((pack) => pack.installed && pack.attribution)
    .map((pack) => ({ prefix: pack.prefix, name: pack.name, author: pack.author.name, license: pack.license.title, url: pack.license.url }));
}

export function stageText(stage: PackStage | null): string {
  switch (stage) {
    case null:
    case 'download':
      return 'Downloading…';
    case 'verify':
      return 'Checking…';
    case 'extract':
      return 'Unpacking…';
    case 'done':
      return 'Finishing…';
    default: {
      const exhaustive: never = stage;
      return exhaustive;
    }
  }
}

const UNREACHABLE = "Can't reach the Companion. Check that Open TouchDeck is running in the tray, then retry.";

/** The most imported packs the Companion keeps, as `icon_pack.too_many` reports. */
export const MAX_IMPORTED = 20;

export function installProblem(error: unknown, name: string): string {
  if (isApiError(error, 'client.unreachable')) return UNREACHABLE;
  if (isApiError(error, 'icon_pack.integrity')) return `${name} didn't match its checksum, so it wasn't installed. Retry later.`;
  if (isApiError(error, 'limits.exceeded') || isApiError(error, 'icon_pack.busy')) return 'Other packs are still downloading. Wait for them, then retry.';
  if (isApiError(error, 'job.cancelled')) return `${name} wasn't installed.`;
  if (isApiError(error, 'icon_pack.download_failed')) return `Couldn't download ${name}. Check the internet connection, then retry.`;
  if (isApiError(error, 'icon_pack.invalid')) return `${name} couldn't be unpacked, so it wasn't installed. Update Open TouchDeck, then retry.`;
  if (isApiError(error, 'internal')) return `Couldn't save ${name} on this PC. Check that the disk has free space, then retry.`;
  return `Couldn't install ${name}. Restart Open TouchDeck, then retry.`;
}

export function removeProblem(error: unknown, name: string): string {
  if (isApiError(error, 'client.unreachable')) return UNREACHABLE;
  if (isApiError(error, 'icon_pack.busy')) return `${name} is still installing. Wait for it, then retry.`;
  return `Couldn't remove ${name}. Restart Open TouchDeck, then retry.`;
}

export function importProblem(error: unknown, file: string): string {
  if (isApiError(error, 'client.unreachable')) return UNREACHABLE;
  if (isApiError(error, 'icon_pack.exists')) return `A pack with the same prefix as “${file}” is already installed or offered above.`;
  if (isApiError(error, 'request.too_large')) return `“${file}” is larger than 32 MB.`;
  if (isApiError(error, 'icon_pack.too_many')) return `You can import up to ${MAX_IMPORTED} packs. Remove one, then import “${file}” again.`;
  return `“${file}” isn't an Iconify JSON pack. Choose an Iconify JSON file, such as icons.json.`;
}

export const LIST_FAILED = "Icon packs didn't load. Restart Open TouchDeck, then retry.";
