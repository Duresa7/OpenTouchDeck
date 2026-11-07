import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import type { DeviceSummary } from '../companion/api.ts';
import { createClient, type Companion } from '../companion/client.ts';
import { createFakeCompanion, FAKE_APP_ID, FAKE_WIFI_CODE, type FakeCompanion } from '../companion/fake.ts';
import { DEMO_APPS, RADIO_CAPS, sampleDevice } from '../companion/samples.ts';
import { windowSession } from '../companion/session.ts';
import { createDevicePairings, offersPairings, pairedCount, type DevicePairingsStore } from './pairings.ts';
import { createWifiPairing } from './wifi.ts';

const BASE = 'http://127.0.0.1:51234';
const USB = { state: 'connected' as const, port: 'COM3', since_ms: 0, protocol: 2 };
const desk: DeviceSummary = { ...sampleDevice('mock0001', 'Desk', USB), capabilities: RADIO_CAPS };
const studio = sampleDevice('mock0002', 'Studio', { ...USB, port: 'COM4' });
const ADDRESS = '192.0.2.21:47820';

let fake: FakeCompanion;
let companion: Companion;
let stores: DevicePairingsStore[];

function start(apps = DEMO_APPS, paired = false) {
  fake = createFakeCompanion({
    devices: [desk, studio],
    devicePairings: { seed: { mock0001: apps } },
    wifi: {
      discovered: [{ device: 'mock0001', name: 'Desk', address: ADDRESS, paired: apps.length }],
      pairings: paired ? [{ device: 'mock0001', address: ADDRESS, name: 'Desk', confirmed: true, paired_ms: 0, problem: null, removing: false }] : [],
    },
  });
  vi.stubGlobal('fetch', vi.fn(fake.fetch));
  companion = createClient(BASE, windowSession(fake.bridge));
}

function pairingsFor(device: DeviceSummary, changed?: () => void): DevicePairingsStore {
  const store = createDevicePairings({ companion, device, changed });
  stores.push(store);
  return store;
}

const pairingCalls = () => vi.mocked(fetch).mock.calls.filter(([input]) => String(input instanceof Request ? input.url : input).includes('/pairings')).length;

beforeEach(() => {
  stores = [];
  start();
});

afterEach(() => {
  for (const store of stores) store.dispose();
  vi.unstubAllGlobals();
});

describe('Pairings in words', () => {
  test('only the radio build keeps Pairings', () => {
    expect(offersPairings(desk)).toBe(true);
    expect(offersPairings(studio)).toBe(false);
    expect(offersPairings({ capabilities: { ...RADIO_CAPS, wifi: false } })).toBe(false);
  });

  test('the count reads as n of the most', () => {
    expect(pairedCount({ pairings: [], max: 4, pairing: false, locked_ms: 0 })).toBe('0 of 4');
  });
});

describe('the list over USB', () => {
  test('lists the apps oldest first, with this app marked', async () => {
    start(DEMO_APPS, true);
    const store = pairingsFor(desk);
    await store.load();
    const shown = store.getState().pairings!;
    expect(shown.pairings.map((p) => p.name)).toEqual(['Open TouchDeck on Laptop', 'Open TouchDeck on Studio PC', 'Open TouchDeck on Workstation']);
    expect(shown.pairings.map((p) => p.this_app)).toEqual([false, false, true]);
    expect(store.count()).toBe(3);
    // No key, ever: each entry is its id, its name and whether it is this app.
    expect(Object.keys(shown.pairings[0]).sort()).toEqual(['app', 'name', 'this_app']);
  });

  test('forgets one app, then every one after the confirm', async () => {
    const changed = vi.fn();
    const store = pairingsFor(desk, changed);
    await store.load();
    expect(await store.forget(DEMO_APPS[0].app)).toBe(true);
    expect(store.getState().pairings!.pairings.map((p) => p.app)).toEqual([DEMO_APPS[1].app]);
    expect(await store.forgetAll()).toBe(true);
    expect(store.getState().pairings!.pairings).toEqual([]);
    expect(changed).toHaveBeenCalledTimes(2);
  });

  test("forgetting this app's Pairing drops its key here too", async () => {
    start([], true);
    const store = pairingsFor(desk);
    await store.load();
    await store.forget(FAKE_APP_ID);
    expect(fake.wifi.pairings()).toEqual([]);
  });

  test('a lost answer says so, and the next read finishes the forget', async () => {
    start([], true);
    const store = pairingsFor(desk);
    await store.load();
    fake.devicePairings.loseNextForget('mock0001');
    expect(await store.forget(FAKE_APP_ID)).toBe(false);
    // The read right after sends it again and finds it gone, so the error clears.
    expect(store.getState().error).toBeNull();
    expect(store.getState().pairings!.pairings).toEqual([]);
    expect(fake.wifi.pairings()).toEqual([]);
  });

  test('until the Device answers, this app reads as being removed', async () => {
    start([], true);
    const store = pairingsFor(desk);
    await store.load();
    fake.devicePairings.loseNextForget('mock0001');
    // Off USB the read that would finish it can't run.
    vi.mocked(fetch).mockImplementationOnce(fake.fetch).mockImplementationOnce(() => Promise.reject(new TypeError('offline')));
    await store.forget(FAKE_APP_ID);
    expect(store.getState().error).toBe("Desk didn't answer, so the app may still be paired. Plug Desk in again to finish.");
    expect(fake.wifi.pairings()[0].removing).toBe(true);
    await store.load();
    expect(fake.wifi.pairings()).toEqual([]);
  });

  test('a failed erase asks to try again', async () => {
    const store = pairingsFor(desk);
    await store.load();
    fake.devicePairings.failStorage(true);
    expect(await store.forget(DEMO_APPS[0].app)).toBe(false);
    expect(store.getState().error).toBe("Desk couldn't erase that. Try again.");
  });
});

describe('what is never sent', () => {
  test('a normal build or a Device on Wi-Fi hears nothing', async () => {
    await pairingsFor(studio).load();
    const onWifi = pairingsFor({ ...desk, carrier: 'wifi', link: { ...USB, port: ADDRESS } });
    await onWifi.load();
    expect(await onWifi.forgetAll()).toBe(false);
    expect(pairingCalls()).toBe(0);
  });

  test('old Firmware that answers needs-firmware says so', async () => {
    const store = pairingsFor({ ...desk, capabilities: { ...RADIO_CAPS, wifi: true } });
    vi.mocked(fetch).mockImplementationOnce(() =>
      Promise.resolve(new Response(JSON.stringify({ error: { code: 'device.needs_firmware', message: 'x', details: { field: 'pairings', version: null } } }), { status: 409 })),
    );
    await store.load();
    expect(store.getState().needs).toBe(true);
  });
});

describe('the pairing flow', () => {
  test('a Device that lists four apps over USB gets the four-apps advice', async () => {
    start([...DEMO_APPS, { app: '3c'.repeat(16), name: 'Laptop' }, { app: '4d'.repeat(16), name: 'Tablet' }]);
    const store = pairingsFor(desk);
    await store.load();
    const wifi = createWifiPairing({ companion, device: () => desk, paired: () => store.count() });
    await wifi.getState().load();
    await wifi.getState().begin();
    expect(wifi.getState().problem).toBe('Four apps are paired. Forget one first: on Desk, use the paired apps screen, or plug it in with a USB cable.');
    // Forgetting one makes room, and pairing works.
    await store.forget(DEMO_APPS[0].app);
    await wifi.getState().cancel();
    await wifi.getState().begin();
    expect(wifi.getState().problem).toBeNull();
    await wifi.getState().start();
    await wifi.getState().submit(FAKE_WIFI_CODE);
    expect(wifi.getState().pairing?.device).toBe('mock0001');
  });
});
