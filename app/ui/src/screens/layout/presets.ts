import type { DeviceSummary, PresetTile } from '../../lib/companion/api.ts';
import { useDevices } from '../../lib/companion/devices.ts';
import { needsFirmware } from '../../lib/device/needs-firmware.ts';
import { useUi } from '../../state/ui.ts';
import { useOfferedFirmware } from './device.ts';

/** The Device the Layout screen edits: the one Home opened, or the first named one. */
export function currentDevice(devices: readonly DeviceSummary[], deviceId: string | null): DeviceSummary | undefined {
  return deviceId ? devices.find((d) => d.id === deviceId) : devices.find((d) => d.name !== null);
}

/**
 * Needs Firmware {x.y} for a bar or now-playing Preset the current Device can't draw. Those are placed anyway and
 * wait for the Firmware, while a graph Preset is placed as its value instead.
 */
export function usePresetNeeds(): (preset: PresetTile) => string | null {
  const deviceId = useUi((s) => s.deviceId);
  const device = useDevices((s) => currentDevice(s.devices, deviceId));
  const offered = useOfferedFirmware({ firmware: device?.firmware ?? null });
  return (preset) => (device && (preset.render === 'bar' || preset.render === 'now-playing') ? needsFirmware(device, preset.render, offered) : null);
}
