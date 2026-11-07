import type { DeviceSummary } from '../../lib/companion/api.ts';
import { escapeInvisible } from '../../lib/layout/actions.ts';

export interface Fact {
  key: 'product' | 'proto' | 'elf_sha256' | 'version' | 'running' | 'sinks' | 'recovery' | 'layout';
  label: string;
  value: string | null;
}

export const nameOf = (device: DeviceSummary) => device.name ?? 'Unnamed Device';

const yesNo = (value: boolean) => (value ? 'Yes' : 'No');

const shown = (value: string | null) => value && escapeInvisible(value);

export function factsOf(device: DeviceSummary): Fact[] {
  const { firmware, diagnostics } = device;
  return [
    { key: 'product', label: 'Product', value: shown(diagnostics && diagnostics.product) },
    { key: 'proto', label: 'Protocol', value: firmware && String(firmware.protocol) },
    { key: 'elf_sha256', label: 'Firmware build', value: shown(firmware && firmware.build) },
    { key: 'version', label: 'Firmware version', value: shown(firmware && firmware.version) },
    { key: 'running', label: 'Running from', value: shown(diagnostics && diagnostics.running_from) },
    { key: 'sinks', label: 'Sends keys through', value: diagnostics && diagnostics.sinks.map(escapeInvisible).join(', ') },
    { key: 'recovery', label: 'Safe mode', value: firmware && yesNo(firmware.recovery) },
    { key: 'layout', label: 'Current Layout', value: shown(diagnostics && diagnostics.current_layout) },
  ];
}

export function connectionText({ device, demo }: { device: DeviceSummary; demo: boolean }): string {
  const name = nameOf(device);
  const { link } = device;
  switch (link.state) {
    case 'connected':
      if (demo) return `${name} is the demo Device.`;
      return device.carrier === 'wifi' ? `${name} is on Wi-Fi at ${link.port}.` : `${name} is on ${link.port}.`;
    case 'connecting':
      return 'Connecting…';
    case 'busy':
      return `Another program is using ${name}. Close it, and ${name} reconnects on its own.`;
    case 'unresponsive':
      return `${name} didn't answer. Unplug it and plug it back in.`;
    case 'lost':
      return `Lost ${name}. Check the USB cable. It reconnects on its own.`;
    case 'updating':
      return `${name} is updating. Wait for it to finish.`;
    case 'offline':
    case 'disconnected':
      return `${name} isn't connected.`;
    default: {
      const _exhaustive: never = link;
      return _exhaustive;
    }
  }
}

export function lastError(device: DeviceSummary): string | null {
  const { link } = device;
  return (link.state === 'lost' || link.state === 'busy') && link.detail ? link.detail : null;
}
