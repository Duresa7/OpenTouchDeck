import { copyFile, lstat, mkdtemp, open, readdir, rm, stat } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { compareVersions, verifyFile } from './signature';
import {
  DOWNLOADS, HELPER, MAX_PAGE, MAX_PAGES, MAX_SIGNATURE, PAGE_SIZE, RELEASES_API, WINDOWS_11_BUILD,
  helperArgs, selectNextApp, signedFields, type NextApp,
} from './bridge';
import type { UpdateCheck, UpdateDownload, UpdateInstall, UpdateProgress } from './types';

export const UPDATE_FEED = 'https://github.com/Duresa7/OpenTouchDeck/releases/latest/download/latest.json';
export const INSTALLER_ARGS = ['/S', '--updated', '--force-run'];
const MAX_DOWNLOAD = 256 * 1024 * 1024;
// GitHub's API refuses a request without a User-Agent. This one names the app and nothing about the PC or the person.
const GITHUB_HEADERS = { Accept: 'application/vnd.github+json', 'User-Agent': 'OpenTouchDeck-updater', 'X-GitHub-Api-Version': '2022-11-28' };
class InvalidDownload extends Error {}
class SignatureFailure extends Error {}
export type SystemCall = (method: string, path: string) => Promise<{ status: number; body: unknown }>;
/** What the bridge to 2.0 needs. Only a packaged Windows app has it. */
export type BridgeOptions = {
  /** The Windows build number, or null when unknown. 2.0 is offered only from build 22000 (Windows 11). */
  windowsBuild: number | null;
  /** The installed app's folder, which bridge.ps1 hands to the MSI. */
  installDir: string;
  /** The app data folder. bridge.ps1 writes its log and result to `bridge` there and never touches the rest. */
  dataDir: string;
  /** bridge.ps1 in the installed resources. It is copied next to the MSI before it runs. */
  helper: string;
  /** powershell.exe. */
  powershell: string;
  /** This process. bridge.ps1 waits for it to exit before it touches the install folder. */
  pid: number;
};
type Options = {
  publicKey: string; current: string; runtimeDir: string; feed?: string; allowLoopback?: boolean;
  /** GitHub's release list and its download base, which the bridge reads. Tests point both at a loopback server. */
  releases?: string; downloads?: string; bridge?: BridgeOptions;
  system: () => SystemCall; launch: (path: string, args: string[]) => Promise<void>; quit: () => void;
};
/** An installer to run. `next` marks the bridge: a 2.x MSI, whose signature must also name its file. */
type Release = { version: string; url: string; signature: string; size?: number; file: string; next?: NextApp };

/**
 * The feed, the release list and their download base. An unpackaged build alone honours the test overrides:
 * `TOUCHDECK_UPDATE_TEST_FEED` for latest.json and `TOUCHDECK_UPDATE_TEST_GITHUB` for a loopback stand-in of
 * github.com and its API.
 */
export function feedSettings(packaged: boolean, env = process.env) {
  const loopback = (value: string) => {
    const u = new URL(value);
    if (u.protocol !== 'http:' || u.hostname !== '127.0.0.1' || u.username || u.password) throw new Error('Invalid test feed');
    return u;
  };
  const feed = !packaged && env.TOUCHDECK_UPDATE_TEST_FEED ? loopback(env.TOUCHDECK_UPDATE_TEST_FEED).href : undefined;
  const github = !packaged && env.TOUCHDECK_UPDATE_TEST_GITHUB ? loopback(env.TOUCHDECK_UPDATE_TEST_GITHUB).origin : undefined;
  return {
    feed: feed ?? UPDATE_FEED,
    releases: github ? `${github}/repos/Duresa7/OpenTouchDeck/releases` : RELEASES_API,
    downloads: github ? `${github}/Duresa7/OpenTouchDeck/releases/download` : DOWNLOADS,
    allowLoopback: !!(feed || github),
  };
}

function optionalText(value: unknown): string | null { return typeof value === 'string' ? value : null; }

function field(value: unknown, name: string): unknown {
  return typeof value === 'object' && value !== null ? Reflect.get(value, name) : undefined;
}

function failedCheck(error: unknown): UpdateCheck {
  return { state: error instanceof TypeError || (error instanceof Error && error.name === 'TimeoutError') ? 'offline' : 'failed' };
}

export class Updater {
  private release?: Release;
  private ready?: { path: string; release: Release };
  private active = false;
  private staging?: string;
  private readonly started: Promise<void>;
  constructor(private readonly options: Options) {
    this.started = this.removeLeftovers();
    void this.started.catch(() => {});
  }

  private async validateFolders(staging?: string) {
    for (const path of [this.options.runtimeDir, ...(staging ? [staging] : [])]) {
      const entry = await lstat(path);
      if (entry.isSymbolicLink() || !entry.isDirectory()) throw new Error('Unsafe staging folder');
    }
  }

  private async removeLeftovers() {
    await this.validateFolders();
    for (const entry of await readdir(this.options.runtimeDir, { withFileTypes: true })) {
      if (entry.name.startsWith('update-') && (entry.isDirectory() || entry.isSymbolicLink())) {
        await rm(join(this.options.runtimeDir, entry.name), { recursive: true, force: true });
      }
    }
  }

  private async cleanup() {
    this.ready = undefined;
    if (this.staging) {
      await rm(this.staging, { recursive: true, force: true });
      this.staging = undefined;
    }
  }

  private validateUrl(value: string): URL {
    const url = new URL(value);
    if (url.username || url.password || (url.protocol !== 'https:' &&
        !(this.options.allowLoopback && url.protocol === 'http:' && url.hostname === '127.0.0.1'))) throw new Error('Unsafe update URL');
    return url;
  }

  private async response(value: string, signal: AbortSignal, headers: Record<string, string> = {}): Promise<{ body: ReadableStream<Uint8Array>; headers: Headers }> {
    let url = this.validateUrl(value);
    for (let i = 0; i < 6; i++) {
      const res = await fetch(url, { redirect: 'manual', signal, headers: { ...headers, 'Accept-Encoding': 'identity' } });
      if ([301, 302, 303, 307, 308].includes(res.status)) {
        await res.body?.cancel();
        url = this.validateUrl(new URL(res.headers.get('location') ?? '', url).href);
        continue;
      }
      if (!res.ok || !res.body || res.headers.get('content-encoding')) { await res.body?.cancel(); throw new Error('Invalid response'); }
      return { body: res.body, headers: res.headers };
    }
    throw new Error('Too many redirects');
  }

  private async small(value: string, limit = 1024 * 1024, headers: Record<string, string> = {}): Promise<Buffer> {
    const res = await this.response(value, AbortSignal.timeout(15000), headers);
    const reader = res.body.getReader();
    const chunks: Uint8Array[] = []; let size = 0;
    try {
      for (;;) {
        const item = await reader.read(); if (item.done) break;
        size += item.value.length;
        if (size > limit) throw new Error('Manifest too large');
        chunks.push(item.value);
      }
      return Buffer.concat(chunks);
    } finally { await reader.cancel().catch(() => {}); }
  }

  /** The 1.x feed: latest.json names the next 1.x installer. */
  private async checkFeed(): Promise<UpdateCheck> {
    const manifest: unknown = JSON.parse((await this.small(this.options.feed ?? UPDATE_FEED)).toString('utf8'));
    const version = field(manifest, 'version');
    if (typeof version !== 'string') throw new Error('Invalid manifest');
    if (compareVersions(version, this.options.current) <= 0) return { state: 'none' };
    const entry = field(field(manifest, 'platforms'), 'windows-x86_64');
    const url = field(entry, 'url'), signature = field(entry, 'signature'), size = field(entry, 'size');
    if (!entry || typeof url !== 'string' || typeof signature !== 'string' || signature.length > 16384 ||
        (size !== undefined && (typeof size !== 'number' || !Number.isSafeInteger(size) || size <= 0 || size > MAX_DOWNLOAD))) throw new Error('Invalid platform');
    this.validateUrl(url);
    this.release = { version, url, signature, size, file: 'setup.exe' };
    return { state: 'available', version, current: this.options.current,
      notes: optionalText(field(manifest, 'notes')), date: optionalText(field(manifest, 'pub_date')) };
  }

  /** The newest stable 2.x release, from the same release list and rules as 2.0's own updater. */
  private async nextApp(): Promise<NextApp | null> {
    const pages: unknown[] = [];
    for (let page = 1; page <= MAX_PAGES; page++) {
      const list: unknown = JSON.parse((await this.small(
        `${this.options.releases ?? RELEASES_API}?per_page=${PAGE_SIZE}&page=${page}`, MAX_PAGE, GITHUB_HEADERS)).toString('utf8'));
      pages.push(list);
      if (!Array.isArray(list) || list.length < PAGE_SIZE) return selectNextApp(pages, MAX_DOWNLOAD, this.options.downloads ?? DOWNLOADS);
    }
    throw new Error('Too many release pages');
  }

  /**
   * A newer 1.x comes first, so a fix to the bridge itself reaches every copy before it moves anyone. Otherwise a
   * Windows 11 PC is offered the newest 2.x. Windows 10 is never offered 2.x; `needsWindows11` says one exists.
   */
  async check(): Promise<UpdateCheck> {
    if (!this.options.publicKey) return { state: 'disabled' };
    if (this.active) return { state: 'failed' };
    this.active = true;
    try {
      this.release = undefined;
      const feed = await this.checkFeed().catch(failedCheck);
      const bridge = this.options.bridge;
      if (feed.state === 'available' || !bridge) return feed;
      // A failed 2.x lookup never hides the 1.x result.
      const next = await this.nextApp().catch(() => null);
      if (!next) return feed;
      if (bridge.windowsBuild !== null && bridge.windowsBuild >= WINDOWS_11_BUILD) {
        this.validateUrl(next.msi.url); this.validateUrl(next.signature.url);
        this.release = { version: next.version, url: next.msi.url, signature: '', size: next.msi.size, file: next.msi.name, next };
        return { state: 'available', version: next.version, current: this.options.current, notes: next.notes, date: next.date, bridge: true };
      }
      return feed.state === 'none' ? { state: 'none', needsWindows11: next.version } : feed;
    } catch (error) { return failedCheck(error); }
    finally { this.active = false; }
  }

  /** Checks the signature, the signed version and, for the bridge, the signed file name. Returns the SHA-256 it read. */
  private async verify(path: string, release: Release): Promise<string> {
    try {
      const { comment, sha256 } = await verifyFile(path, release.signature, this.options.publicKey);
      const signed = signedFields(comment);
      if (!signed.version || signed.version.replace(/^v/, '') !== release.version.replace(/^v/, '')) throw new Error('Signed version mismatch');
      if (release.next && signed.file !== release.file) throw new Error('Signed file mismatch');
      return sha256;
    } catch { throw new SignatureFailure(); }
  }

  async download(progress: (p: UpdateProgress) => void): Promise<UpdateDownload> {
    if (this.active) return { state: 'failed', reason: 'download' };
    this.active = true;
    let reading = false;
    try {
      await this.started;
      await this.cleanup();
      const release = this.release;
      if (!release || !this.options.publicKey) throw new InvalidDownload();
      if (release.next) {
        // The MSI's signature is a release asset of its own, read fresh for each download.
        const wrapped = (await this.small(release.next.signature.url, MAX_SIGNATURE)).toString('utf8').trim();
        if (!wrapped) throw new SignatureFailure();
        release.signature = wrapped;
      }
      await this.validateFolders();
      this.staging = await mkdtemp(join(this.options.runtimeDir, 'update-'));
      const path = join(this.staging, release.file);
      const response = await this.response(release.url, AbortSignal.timeout(5 * 60 * 1000));
      // Lock the body before the next await: fetch cancels an unread body once its Response is garbage collected.
      const reader = response.body.getReader();
      try {
        const length = response.headers.get('content-length');
        const total = length === null ? null : Number(length);
        if (total !== null && (!Number.isSafeInteger(total) || total <= 0 || total > MAX_DOWNLOAD)) throw new InvalidDownload();
        await this.validateFolders(this.staging);
        const file = await open(path, 'wx', 0o600);
        let downloaded = 0;
        reading = true;
        try {
          for (;;) {
            const item = await reader.read(); if (item.done) break;
            downloaded += item.value.length;
            if (downloaded > MAX_DOWNLOAD || (total !== null && downloaded > total) || (release.size !== undefined && downloaded > release.size)) throw new InvalidDownload();
            await file.writeFile(item.value);
            try { progress({ downloaded, total: release.size ?? total }); } catch { }
          }
          if ((total !== null && downloaded !== total) || (release.size !== undefined && downloaded !== release.size)) throw new SignatureFailure();
        } finally { await file.close(); }
        reading = false;
      } finally { await reader.cancel().catch(() => {}); }
      await this.verify(path, release);
      this.ready = { path, release };
      return { state: 'ready' };
    } catch (error) {
      await this.cleanup().catch(() => {});
      if (error instanceof SignatureFailure || (reading && !(error instanceof InvalidDownload))) return { state: 'failed', reason: 'signature' };
      return error instanceof TypeError || (error instanceof Error && error.name === 'TimeoutError')
        ? { state: 'offline' } : { state: 'failed', reason: 'download' };
    } finally { this.active = false; }
  }

  async install(): Promise<UpdateInstall> {
    if (this.active || !this.ready) return { state: 'failed', reason: 'installer' };
    this.active = true;
    let lock: string | undefined;
    let system: SystemCall | undefined;
    try {
      system = this.options.system();
      const ready = this.ready;
      const bridge = ready.release.next ? this.options.bridge : undefined;
      if (ready.release.next && !bridge) throw new Error('No bridge on this build');
      await this.validateFolders(dirname(ready.path));
      if ((await stat(ready.path)).size > MAX_DOWNLOAD) throw new Error('Oversized installer');
      await this.verify(ready.path, ready.release);
      // bridge.ps1 runs from the staging folder, because it moves the install folder. Copy it before the Companion
      // stops, so a missing helper never stops it for nothing.
      const script = join(dirname(ready.path), HELPER);
      if (bridge) await copyFile(bridge.helper, script);
      const acquired = await system('POST', '/system/lock');
      const error = field(acquired.body, 'error');
      if (acquired.status === 409 && field(error, 'code') === 'system.busy') {
        const device = field(field(error, 'details'), 'device');
        return { state: 'failed', reason: 'busy', device: typeof device === 'string' ? device : null };
      }
      const acquiredLock = field(acquired.body, 'lock');
      if (acquired.status !== 200 || typeof acquiredLock !== 'string' || !acquiredLock) throw new Error('Lock unavailable');
      lock = acquiredLock;
      const renewed = await system('PUT', `/system/lock/${encodeURIComponent(lock)}`);
      if (renewed.status < 200 || renewed.status >= 300) throw new Error('Lock lost');
      const stopped = await system('POST', '/system/shutdown');
      if (stopped.status < 200 || stopped.status >= 300) throw new Error('Shutdown failed');
      await this.validateFolders(dirname(ready.path));
      if ((await stat(ready.path)).size > MAX_DOWNLOAD) throw new Error('Oversized installer');
      if (bridge) {
        // bridge.ps1 installs only the bytes whose SHA-256 this last signature check read.
        const sha256 = await this.verify(ready.path, ready.release);
        await this.options.launch(bridge.powershell, helperArgs({
          script, msi: ready.path, sha256, installDir: bridge.installDir, dataDir: bridge.dataDir, waitPid: bridge.pid,
        }));
      } else {
        await this.verify(ready.path, ready.release);
        await this.options.launch(ready.path, [...INSTALLER_ARGS]);
      }
      this.options.quit();
      return await new Promise<never>(() => {});
    } catch {
      await this.cleanup().catch(() => {});
      return { state: 'failed', reason: 'installer' };
    } finally {
      if (lock && system) await system('DELETE', `/system/lock/${encodeURIComponent(lock)}`).catch(() => {});
      this.active = false;
    }
  }
}
