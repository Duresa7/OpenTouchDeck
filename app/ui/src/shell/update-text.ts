import type { CheckState, FailedReason, UpdateStep } from '../state/updates.ts';

type Downloading = Extract<UpdateStep, { kind: 'downloading' }>;

const KB = 1024;
const MB = 1024 * 1024;

export const failureText = {
  download: "Couldn't download the update. Check your internet connection, then retry.",
  signature: "The download didn't match the project's signing key, so it wasn't installed. Retry later.",
  offline: "You're offline. Connect to the internet, then retry.",
  installer: "Couldn't start the installer. Retry, or download it from the releases page.",
} satisfies Record<FailedReason, string>;

/** Release notes as plain lines for the change list: blank lines and Markdown headings go, bullet marks are dropped. */
export function changeLines(notes: string | null): string[] {
  return (notes ?? '')
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line !== '' && !line.startsWith('#'))
    .map((line) => line.replace(/^[-*+•](?:\s+|$)/, ''))
    .filter((line) => line !== '');
}

/** Whole percent, never 100 before the last byte. Null when the size isn't known. */
export function percent({ downloaded, total }: Downloading): number | null {
  if (!total) return null;
  return Math.min(100, Math.floor((downloaded / total) * 100));
}

function megabytes(bytes: number): string {
  const mb = bytes / MB;
  return mb < 10 ? mb.toFixed(1) : String(Math.round(mb));
}

export function downloadingText(step: Downloading): string {
  const pct = percent(step);
  return pct === null ? `Downloading… ${Math.round(step.downloaded / KB)} KB` : `Downloading… ${pct}%`;
}

/** The megabytes row under the progress bar, or null when the size isn't known. */
export function sizeText(step: Downloading): string | null {
  return step.total ? `${megabytes(step.downloaded)} MB of ${megabytes(step.total)} MB` : null;
}

export function iconLabel(step: UpdateStep): string {
  switch (step.kind) {
    case 'downloading':
      return downloadingText(step);
    case 'ready':
    case 'busy':
    case 'installing':
      return 'Ready to restart';
    case 'offer':
    case 'failed':
      return 'Update available';
    default: {
      const _exhaustive: never = step;
      return _exhaustive;
    }
  }
}

/** What moving to 2.x means, under the offer's title. 2.x replaces this app; nothing the person made is lost. */
export function bridgeText(offer: { version: string; current: string }): string {
  return `You have ${offer.current}. Open TouchDeck ${offer.version} is the new app for Windows 11 and replaces this one. Layouts, settings, Devices and Pairings stay.`;
}

/** The Restart step of a move to 2.x. */
export const bridgeReadyText = (version: string) =>
  `Open TouchDeck closes, installs ${version} and opens it. Layouts, settings, Devices and Pairings stay.`;

/** Settings → About on Windows 10, once a 2.x release exists. */
export const needsWindows11Text = (version: string) =>
  `Open TouchDeck ${version} needs Windows 11. This PC keeps Open TouchDeck 1.x, which still gets updates.`;

export const waitText = (device: string | null) => `Wait for ${device ?? 'the Device'} to finish sending.`;

/** The line Settings → About shows for a check, or null when it shows nothing. */
export function checkText(check: CheckState): string | null {
  switch (check) {
    case 'checking':
      return 'Checking…';
    case 'current':
      return 'Up to date';
    case 'offline':
      return failureText.offline;
    case 'failed':
      return "Can't reach the update server. Check your internet connection, then retry.";
    case 'idle':
    case 'found':
    case 'disabled':
      return null;
    default: {
      const _exhaustive: never = check;
      return _exhaustive;
    }
  }
}
