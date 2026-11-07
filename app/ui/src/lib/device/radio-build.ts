import type { DeviceSummary, FirmwareBuild, FirmwareChannel } from '../companion/api.ts';
import { newerFirmware } from './needs-firmware.ts';

/** The build a Device runs. Firmware that doesn't say runs the normal build. */
export const channelOf = (device: Pick<DeviceSummary, 'firmware'>): FirmwareChannel => device.firmware?.channel ?? 'normal';

/** The version of the build this Companion carries for a channel, or null when it carries none. */
export function bundledVersion(bundled: FirmwareBuild | null, channel: FirmwareChannel): string | null {
  if (!bundled) return null;
  return channel === 'radio' ? (bundled.radio?.version ?? null) : bundled.version;
}

/** The newer Firmware this Companion offers a Device: always from the build the Device already runs. */
export function offeredFor(device: Pick<DeviceSummary, 'firmware'>, bundled: FirmwareBuild | null): string | null {
  return newerFirmware(device, bundledVersion(bundled, channelOf(device)));
}

export interface BuildChoice {
  /** The build the Device runs now. */
  current: FirmwareChannel;
  /** The build a switch installs. */
  target: FirmwareChannel;
  title: string;
  note: string;
  /** The button's words, or null when this Companion carries no copy of the other build. */
  action: string | null;
  missing: string | null;
  confirmTitle: string;
  confirmNote: string;
  /** Switching back keeps the radio setup on the Device, unused, so the note warns. */
  warn: boolean;
}

const TURNS_ON = 'Wi-Fi and Bluetooth, both off until you set them up';

export function buildChoice(device: Pick<DeviceSummary, 'firmware'>, bundled: FirmwareBuild | null, name: string): BuildChoice {
  const current = channelOf(device);
  const target: FirmwareChannel = current === 'radio' ? 'normal' : 'radio';
  const available = bundledVersion(bundled, target) !== null;
  if (current === 'radio') {
    return {
      current,
      target,
      title: 'Radio build',
      note: `Adds ${TURNS_ON}. The normal build has no radio code.`,
      action: available ? 'Switch to the normal build' : null,
      missing: available ? null : "This copy of Open TouchDeck doesn't include the normal build.",
      confirmTitle: `Switch ${name} back to the normal build?`,
      confirmNote: `The Wi-Fi and Bluetooth setup stays stored on ${name} but unused. ${name} restarts once.`,
      warn: true,
    };
  }
  return {
    current,
    target,
    title: 'Normal build',
    note: `The radio build adds ${TURNS_ON}. You can switch back any time.`,
    action: available ? 'Install the radio build' : null,
    missing: available ? null : "This copy of Open TouchDeck doesn't include the radio build.",
    confirmTitle: `Install the radio build on ${name}?`,
    confirmNote: `It adds ${TURNS_ON}. ${name} restarts once.`,
    warn: false,
  };
}

/** Onboarding offers Wi-Fi setup only to a Device that runs the radio build. */
export const offersWifi = (device: Pick<DeviceSummary, 'firmware'>): boolean => channelOf(device) === 'radio';
