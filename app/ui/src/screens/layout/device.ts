import { useEffect } from 'react';
import { useStore } from 'zustand';
import { createStore, type StoreApi } from 'zustand/vanilla';
import type { DeviceSummary } from '../../lib/companion/api.ts';
import type { Companion } from '../../lib/companion/client.ts';
import { useCompanion } from '../../lib/companion/connection.ts';
import { jobsStore } from '../../lib/companion/jobs.ts';
import { createBluetooth, type BluetoothStore } from '../../lib/device/bluetooth.ts';
import { createFirmwareUpdate, type FirmwareUpdateStore } from '../../lib/device/firmware.ts';
import { newerFirmware } from '../../lib/device/needs-firmware.ts';
import { createDeviceSettings, type DeviceSettingsStore } from '../../lib/device/settings.ts';
import { createWifiPairing, type WifiStore } from '../../lib/device/wifi.ts';
import { createWifiSetup, type WifiSetupStore } from '../../lib/device/wifi-setup.ts';

interface DeviceStores {
  settings: DeviceSettingsStore;
  firmware: FirmwareUpdateStore;
  wifi: WifiStore;
  wifiSetup: WifiSetupStore;
  bluetooth: BluetoothStore;
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
    const wifiSetup = createWifiSetup({ companion, device });
    const bluetooth = createBluetooth({ companion, device });
    const firmware = createFirmwareUpdate({
      companion,
      jobs: jobsStore,
      context: () => ({ id: latest.id, device: latest.name ?? 'the Device', version: latest.firmware?.version ?? '' }),
    });
    found = {
      settings,
      firmware,
      wifi: createWifiPairing({ companion, device: () => latest }),
      wifiSetup,
      bluetooth,
      observe(summary) {
        latest = summary;
        settings.observe(summary);
        wifiSetup.observe(summary);
        bluetooth.observe(summary);
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

type Bundled = { state: 'asking' } | { state: 'known'; version: string } | { state: 'unknown' };

const bundled = new WeakMap<Companion, StoreApi<Bundled>>();

function bundledFor(companion: Companion): StoreApi<Bundled> {
  let store = bundled.get(companion);
  if (!store) {
    const asking = createStore<Bundled>()(() => ({ state: 'asking' }));
    companion.firmware.bundled().then(
      (build) => asking.setState({ state: 'known', version: build.version }),
      () => asking.setState({ state: 'unknown' }),
    );
    bundled.set(companion, asking);
    store = asking;
  }
  return store;
}

export function useBundledFirmware(): string | null {
  return useStore(bundledFor(useCompanion()), (s) => (s.state === 'known' ? s.version : null));
}

export function useOfferedFirmware(device: Pick<DeviceSummary, 'firmware'>): string | null {
  const bundled = useBundledFirmware();
  return bundled ? newerFirmware(device, bundled) : (device.firmware?.update_available ?? null);
}
