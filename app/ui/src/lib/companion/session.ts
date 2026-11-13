// Where a client's session token comes from (docs/companion-api.md, "How the Electron
// window gets its session" and "How a browser pairs").

export type UpdateCheck =
  | { state: 'none'; needsWindows11?: string }          // up to date; needsWindows11 names a 2.x this PC can't run
  | { state: 'available'; version: string; current: string; notes: string | null; date: string | null; bridge?: true } // bridge: 2.x replaces this app
  | { state: 'disabled' }                                // source build: no public key was built in
  | { state: 'offline' }
  | { state: 'failed' };
export type UpdateProgress = { downloaded: number; total: number | null };
export type UpdateDownload = { state: 'ready' } | { state: 'offline' } | { state: 'failed'; reason: 'download' | 'signature' };
export type UpdateInstall = { state: 'failed'; reason: 'busy'; device: string | null } | { state: 'failed'; reason: 'installer' };

// What Electron's preload exposes. The renderer never sees the pairing token.
export interface OpenTouchDeckBridge {
  version: 1;
  session(stale?: string): Promise<string | null>;
  onSessionChanged(callback: () => void): void;
  // Every call resolves; none rejects, because Errors lose their fields across contextBridge.
  updates?: {
    check(): Promise<UpdateCheck>;
    download(onProgress: (p: UpdateProgress) => void): Promise<UpdateDownload>;
    // On success, install() never resolves: the app quits and the installer runs.
    install(): Promise<UpdateInstall>;
  };
  // The folder the user picked in the system dialog, or null when they cancelled.
  pickFolder?(): Promise<string | null>;
}

declare global {
  interface Window {
    openTouchDeck?: OpenTouchDeckBridge;
  }
}

export interface Session {
  readonly kind: 'window' | 'browser';
  /** The token to send, loading it first if needed. Null means this client has to pair. */
  token(): Promise<string | null>;
  /** The token loaded last, without waiting. */
  current(): string | null;
  /**
   * After a 401 or close 4401 for `stale`: a new token, or null when this client
   * can't get one by itself. Several callers that saw the same stale token share one renewal.
   */
  renew(stale: string | null): Promise<string | null>;
}

export function windowSession(bridge: OpenTouchDeckBridge): Session {
  let token: string | null = null;
  let loaded = false;
  let pending: Promise<string | null> | null = null;

  const ask = (stale?: string) => {
    pending ??= bridge.session(stale).then(
      (fresh) => {
        token = fresh;
        loaded = true;
        pending = null;
        return fresh;
      },
      (error: unknown) => {
        pending = null;
        throw error;
      },
    );
    return pending;
  };

  bridge.onSessionChanged(() => void ask());

  return {
    kind: 'window',
    token: () => (loaded ? Promise.resolve(token) : ask()),
    current: () => token,
    renew: (stale) => (pending ?? (token !== stale ? Promise.resolve(token) : ask(stale ?? undefined))),
  };
}

export const BROWSER_SESSION_KEY = 'touchdeck.session';

// A paired browser keeps its token in localStorage. A 401 means the Companion
// restarted or revoked it, so the token is cleared and the browser pairs again.
export function browserSession(storage: Pick<Storage, 'getItem' | 'removeItem'>): Session {
  const read = () => storage.getItem(BROWSER_SESSION_KEY);
  return {
    kind: 'browser',
    token: () => Promise.resolve(read()),
    current: read,
    renew: (stale) => {
      if (stale !== null && read() === stale) storage.removeItem(BROWSER_SESSION_KEY);
      return Promise.resolve(read());
    },
  };
}

export function pickSession(target: Window): Session {
  return target.openTouchDeck ? windowSession(target.openTouchDeck) : browserSession(target.localStorage);
}
