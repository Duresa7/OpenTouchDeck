// The bridge to Open TouchDeck 2.0 (docs/packaging.md, "The bridge to 2.0"). 2.0 is a separate app that ships as a
// per-user MSI for Windows 11. This release finds it in the repository's GitHub releases, as 2.0's own updater does,
// and hands the verified MSI to bridge.ps1, which installs it and removes this app.
import { compareVersions } from './signature';

export const RELEASES_API = 'https://api.github.com/repos/Duresa7/OpenTouchDeck/releases';
export const DOWNLOADS = 'https://github.com/Duresa7/OpenTouchDeck/releases/download';
/** 2.0 needs Windows 11, which starts at build 22000. The MSI refuses an older Windows too. */
export const WINDOWS_11_BUILD = 22000;
export const PAGE_SIZE = 100;
export const MAX_PAGES = 10;
export const MAX_PAGE = 2 * 1024 * 1024;
export const MAX_SIGNATURE = 16 * 1024;
/** The helper's file name, next to app.asar in the installed resources folder. */
export const HELPER = 'bridge.ps1';
/** Where Windows keeps the 64-bit PowerShell 5.1 that every Windows 10 and 11 has. */
export const POWERSHELL = 'WindowsPowerShell\\v1.0\\powershell.exe';

export type NextApp = {
  version: string; tag: string; notes: string | null; date: string | null;
  msi: { name: string; url: string; size: number }; signature: { url: string; size: number };
};

/** The Windows build number from `os.release()` ("10.0.22631" is 22631), or null on another OS or an odd value. */
export function windowsBuild(release: string, platform: string): number | null {
  if (platform !== 'win32') return null;
  const m = /^10\.0\.(\d{1,6})(?:\.|$)/.exec(release);
  return m ? Number(m[1]) : null;
}

export function msiName(version: string): string {
  return `OpenTouchDeck-${version}-windows-x64.msi`;
}

function text(value: unknown): string | null {
  return typeof value === 'string' ? value : null;
}

function field(value: unknown, name: string): unknown {
  return typeof value === 'object' && value !== null ? Reflect.get(value, name) : undefined;
}

function stable2(tag: string): string | null {
  const version = tag.startsWith('v') ? tag.slice(1) : tag;
  try { compareVersions(version, '2.0.0'); } catch { return null; }
  return /^2\.\d+\.\d+(?:\+[0-9A-Za-z.-]+)?$/.test(version) ? version : null;
}

/** The one asset with this name, or an error: none, two, an empty or oversized file, or a URL outside this release. */
function asset(release: unknown, tag: string, name: string, max: number, downloads: string) {
  const matches = (Array.isArray(field(release, 'assets')) ? field(release, 'assets') as unknown[] : [])
    .filter(item => field(item, 'name') === name);
  if (matches.length !== 1) throw new Error(`Release ${tag} needs exactly one ${name}`);
  const url = field(matches[0], 'browser_download_url'), size = field(matches[0], 'size');
  if (url !== `${downloads}/${tag}/${name}`) throw new Error(`Unexpected URL for ${name}`);
  if (typeof size !== 'number' || !Number.isSafeInteger(size) || size <= 0 || size > max) throw new Error(`Invalid size for ${name}`);
  return { url, size };
}

/**
 * The newest stable 2.x release among `pages` of GitHub's release list, with its MSI and signature, or null when there
 * is none. Drafts, pre-releases and other major versions never count, as in 2.0's own updater (`td-updater`).
 */
export function selectNextApp(pages: unknown[], maxMsi: number, downloads = DOWNLOADS): NextApp | null {
  let best: { version: string; release: unknown; tag: string } | null = null;
  for (const page of pages) {
    if (!Array.isArray(page)) throw new Error('Invalid release list');
    for (const release of page) {
      const tag = field(release, 'tag_name');
      if (typeof tag !== 'string' || field(release, 'draft') !== false || field(release, 'prerelease') !== false) continue;
      const version = stable2(tag);
      if (version && (!best || compareVersions(version, best.version) > 0)) best = { version, release, tag };
    }
  }
  if (!best) return null;
  const name = msiName(best.version);
  return {
    version: best.version, tag: best.tag,
    notes: text(field(best.release, 'body')), date: text(field(best.release, 'published_at')),
    msi: { name, ...asset(best.release, best.tag, name, maxMsi, downloads) },
    signature: asset(best.release, best.tag, `${name}.sig`, MAX_SIGNATURE, downloads),
  };
}

/** The signed `file:` and `version:` fields of a minisign trusted comment. */
export function signedFields(comment: string): { file?: string; version?: string } {
  const fields: Record<string, string> = {};
  for (const part of comment.split('\t')) {
    const at = part.indexOf(':');
    if (at > 0) fields[part.slice(0, at)] = part.slice(at + 1);
  }
  return { file: fields.file, version: fields.version };
}

/**
 * What 1.x says at its next start after bridge.ps1 put it back, from the result bridge.ps1 wrote, or null when there
 * is nothing to say. The strings are in docs/copy.md.
 */
export function bridgeNotice(result: unknown, folder: string): { message: string; detail: string } | null {
  const kept = 'Open TouchDeck 1.x is still installed, with your Layouts and settings.';
  switch (field(result, 'status')) {
    case 'cancelled':
      return { message: 'The update to Open TouchDeck 2.0 was cancelled.', detail: `${kept} Update again from Settings → About when you're ready.` };
    case 'failed':
    case 'refused':
      return { message: "Couldn't update to Open TouchDeck 2.0.", detail: `${kept} Retry from Settings → About. The log is in ${folder}.` };
    default:
      return null;
  }
}

export type HelperRun ={ script: string; msi: string; sha256: string; installDir: string; dataDir: string; waitPid: number };

/** The PowerShell command line that runs bridge.ps1. Every value is its own argument; nothing goes through a shell. */
export function helperArgs(run: HelperRun): string[] {
  return ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', run.script,
    '-Msi', run.msi, '-Sha256', run.sha256, '-InstallDir', run.installDir, '-DataDir', run.dataDir,
    '-WaitPid', String(run.waitPid)];
}
