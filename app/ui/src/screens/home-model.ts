import type { DeviceSummary, Layout, LayoutSummary, Orientation } from '../lib/companion/api.ts';
import type { Unchecked } from '../lib/device/controls.ts';

export function statusOf(state: string) {
  switch (state) {
    case 'connected': return { words: 'Connected', live: true };
    case 'connecting': return { words: 'Connecting…', live: false };
    case 'busy': return { words: 'In use by another program', live: false };
    case 'updating': return { words: 'Updating…', live: false };
    case 'restarting': return { words: 'Restarting…', live: false };
    default: return { words: 'Not connected', live: false };
  }
}

/** The link in words, naming Wi-Fi when that's the Carrier. */
export function deviceStatus(device: Pick<DeviceSummary, 'link' | 'carrier'>) {
  const status = statusOf(device.link.state);
  return status.live && device.carrier === 'wifi' ? { ...status, words: 'Connected over Wi-Fi' } : status;
}

export const NEEDS_SEND = 'Controls need Send to Device';
export const CHECKING_CONTROLS = 'Checking controls…';

export function uncheckedText(reason: Unchecked, device: string) {
  switch (reason) {
    case 'read-failed': return `Couldn't check ${device}'s controls. Unplug it and plug it back in.`;
    case 'unsupported': return `Couldn't check ${device}'s controls. Update its Firmware.`;
    default: {
      const exhaustive: never = reason;
      return exhaustive;
    }
  }
}

export const deviceWidth = (count: number) => count === 1 ? 560 : 312;
export const firmwareUpdate = (device: DeviceSummary) => Boolean(device.firmware?.update_available);
export const orientationOf = (device: DeviceSummary): Orientation => device.orientation === 'portrait' ? 'portrait' : 'landscape';
export const shownOrientation = (device: DeviceSummary): Orientation => device.layout?.orientation ?? orientationOf(device);

export const NO_LAYOUT = 'No Layout yet';
export const NO_LAYOUT_HINT = 'Click to make one';

export interface Preview {
  layout: Layout;
  index: number;
}

function isShowing(shown: LayoutSummary | null, layout: Layout): shown is LayoutSummary & { page: number } {
  return shown?.source === 'storage' && shown.page !== null && shown.page < layout.pages.length
    && shown.name === layout.name && shown.orientation === layout.orientation && shown.pages === layout.pages.length;
}

export function previewOf(layout: Layout | null | undefined, shown: LayoutSummary | null = null): Preview | null {
  if (!layout?.pages.length) return null;
  return { layout, index: isShowing(shown, layout) ? shown.page : 0 };
}

export function lastSeen(device: DeviceSummary) {
  if (device.link.state !== 'lost' || device.link.at_ms <= 0) return null;
  const date = new Date(device.link.at_ms);
  return Number.isNaN(date.getTime()) ? null : `Last seen ${date.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}`;
}
