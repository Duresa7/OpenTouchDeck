import { useEffect } from 'react';
import { useStore } from 'zustand';
import { createStore, type StoreApi } from 'zustand/vanilla';
import type { DeviceSummary, FirmwareBuild } from '../../lib/companion/api.ts';
import type { Companion } from '../../lib/companion/client.ts';
import { useCompanion } from '../../lib/companion/connection.ts';
import { jobsStore } from '../../lib/companion/jobs.ts';
import { createFirmwareUpdate, type FirmwareUpdateStore } from '../../lib/device/firmware.ts';
import { offeredFor } from '../../lib/device/radio-build.ts';
import { createDeviceSettings, type DeviceSettingsStore } from '../../lib/device/settings.ts';
import { createWifiPairing, type WifiStore } from '../../lib/device/wifi.ts';

interface DeviceStores {
  settings: DeviceSettingsStore;
  firmware: FirmwareUpdateStore;
  wifi: WifiStore;
  observe(summary: DeviceSummary): void;
}

const stores = new WeakMap<Companion, Map<string, DeviceStores>>();

export function deviceStoresFor(companion: Companion, device: DeviceSummary): DeviceStores {
  let byDevice = stores.get(companion);
  if (!byDevice) {
    byDevice = new Map();
    stores.set(companion, byDevice);
  }
  let found = byDevice.get(device.id);
  if (!found) {
    let latest = device;
    const settings = createDeviceSettings({ companion, device });
    const firmware = createFirmwareUpdate({
      companion,
      jobs: jobsStore,
      context: () => ({ id: latest.id, device: latest.name ?? 'the Device', version: latest.firmware?.version ?? '' }),
    });
    found = {
      settings,
      firmware,
      wifi: createWifiPairing({ companion, device: () => latest }),
      observe(summary) {
        latest = summary;
        settings.observe(summary);
      },
    };
    byDevice.set(device.id, found);
  }
  return found;
}

export function useDeviceStores(device: DeviceSummary): DeviceStores {
  const stores = deviceStoresFor(useCompanion(), device);
  const job = device.link.state === 'updating' ? device.link.job : null;
  useEffect(() => stores.observe(device), [stores, device]);
  useEffect(() => {
    const known = job ? jobsStore.getState().jobs[job] : undefined;
    if (known) stores.firmware.attach(known);
  }, [stores, job]);
  return stores;
}

type Bundled = { state: 'asking' } | { state: 'known'; build: FirmwareBuild } | { state: 'unknown' };

const bundled = new WeakMap<Companion, StoreApi<Bundled>>();

function bundledFor(companion: Companion): StoreApi<Bundled> {
  let store = bundled.get(companion);
  if (!store) {
    const asking = createStore<Bundled>()(() => ({ state: 'asking' }));
    companion.firmware.bundled().then(
      (build) => asking.setState({ state: 'known', build }),
      () => asking.setState({ state: 'unknown' }),
    );
    bundled.set(companion, asking);
    store = asking;
  }
  return store;
}

/** The Firmware this Companion carries: the normal build, and the radio build when it carries one. */
export function useBundledBuild(): FirmwareBuild | null {
  return useStore(bundledFor(useCompanion()), (s) => (s.state === 'known' ? s.build : null));
}

/** The newer Firmware offered to a Device, always of the build it runs. */
export function useOfferedFirmware(device: Pick<DeviceSummary, 'firmware'>): string | null {
  const bundled = useBundledBuild();
  return bundled ? offeredFor(device, bundled) : (device.firmware?.update_available ?? null);
}
