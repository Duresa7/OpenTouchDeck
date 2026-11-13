export type UpdateCheck =
  | { state: 'none'; needsWindows11?: string }
  | { state: 'available'; version: string; current: string; notes: string | null; date: string | null; bridge?: true }
  | { state: 'disabled' }
  | { state: 'offline' }
  | { state: 'failed' };
export type UpdateProgress = { downloaded: number; total: number | null };
export type UpdateDownload = { state: 'ready' } | { state: 'offline' } | { state: 'failed'; reason: 'download' | 'signature' };
export type UpdateInstall = { state: 'failed'; reason: 'busy'; device: string | null } | { state: 'failed'; reason: 'installer' };
