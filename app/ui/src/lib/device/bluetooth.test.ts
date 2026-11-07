import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import type { DeviceBluetooth, DeviceSummary } from '../companion/api.ts';
import { createClient, type Companion } from '../companion/client.ts';
import { createFakeCompanion, type FakeCompanion } from '../companion/fake.ts';
import { DEMO_BONDS, RADIO_CAPS, sampleDevice } from '../companion/samples.ts';
import { windowSession } from '../companion/session.ts';
import { bluetoothStatusText, countdown, createBluetooth, offersBluetooth, type BluetoothStore } from './bluetooth.ts';

const BASE = 'http://127.0.0.1:51234';
const USB = { state: 'connected' as const, port: 'COM3', since_ms: 0, protocol: 2 };
const desk: DeviceSummary = { ...sampleDevice('mock0001', 'Desk', USB), capabilities: RADIO_CAPS };
const studio = sampleDevice('mock0002', 'Studio', { ...USB, port: 'COM4' });
const NEW_HOST = '00:00:5e:00:53:03';

let fake: FakeCompanion;
let companion: Companion;
let stores: BluetoothStore[];

function keyboardFor(device: DeviceSummary, pollMs = 1000): BluetoothStore {
  const store = createBluetooth({ companion, device, pollMs });
  stores.push(store);
  return store;
}

function start(seed: Record<string, { enabled?: boolean; bonds?: DeviceBluetooth['bonds'] }> = { mock0001: { enabled: true, bonds: DEMO_BONDS } }) {
  fake = createFakeCompanion({ devices: [desk, studio], bluetooth: { seed } });
  vi.stubGlobal('fetch', vi.fn(fake.fetch));
  companion = createClient(BASE, windowSession(fake.bridge));
}

const bluetoothCalls = () => vi.mocked(fetch).mock.calls.filter(([input]) => String(input instanceof Request ? input.url : input).includes('/bluetooth')).length;

beforeEach(() => {
  stores = [];
  start();
});

afterEach(() => {
  for (const store of stores) store.dispose();
  vi.unstubAllGlobals();
});

const state = (patch: Partial<DeviceBluetooth> = {}): DeviceBluetooth => ({
  enabled: true,
  state: 'on',
  pairing: false,
  pairing_ms: 0,
  passkey: false,
  advertising: 'off',
  bonds: [],
  bonds_max: 4,
  battery: 100,
  ...patch,
});

describe('the Bluetooth keyboard in words', () => {
  test('only a Device whose Firmware advertises ble_hid offers it', () => {
    expect(offersBluetooth(desk)).toBe(true);
    expect(offersBluetooth(studio)).toBe(false);
    expect(offersBluetooth({ capabilities: { ...RADIO_CAPS, ble_hid: 0 } })).toBe(false);
  });

  test('the countdown rounds up and never goes below 0:00', () => {
    expect(countdown(120_000)).toBe('2:00');
    expect(countdown(119_001)).toBe('2:00');
    expect(countdown(65_000)).toBe('1:05');
    expect(countdown(1)).toBe('0:01');
    expect(countdown(-5)).toBe('0:00');
  });

  test('one line for each state, and the code stays on the Device', () => {
    expect(bluetoothStatusText(state({ enabled: false, state: 'off' }), 'Desk')).toBe('Off. Turn it on to type on a computer or phone with Desk.');
    expect(bluetoothStatusText(state({ state: 'starting' }), 'Desk')).toBe('Starting…');
    expect(bluetoothStatusText(state({ state: 'failed' }), 'Desk')).toBe("Bluetooth didn't start on Desk. Restart Desk to try again.");
    expect(bluetoothStatusText(state({ pairing: true, pairing_ms: 90_000 }), 'Desk')).toBe('Ready to pair. Waiting for a computer or phone…');
    expect(bluetoothStatusText(state({ pairing: true, pairing_ms: 90_000, passkey: true }), 'Desk')).toBe('Desk shows a 6-digit code. Type it on the computer or phone.');
    expect(bluetoothStatusText(state({ bonds: DEMO_BONDS }), 'Desk')).toBe('On. Connected to a computer or phone.');
    expect(bluetoothStatusText(state({ bonds: [DEMO_BONDS[1]] }), 'Desk')).toBe('On. Waiting for a paired computer or phone.');
    expect(bluetoothStatusText(state(), 'Desk')).toBe('On. Pair a computer or phone to type with Desk.');
  });
});

describe('the Bluetooth keyboard store', () => {
  test('reads the keyboard, turns it off and back on, and keeps the bonds', async () => {
    const store = keyboardFor(desk);
    await store.load();
    expect(store.getState()).toMatchObject({ load: 'ready', bluetooth: { enabled: true, state: 'on', bonds: DEMO_BONDS, bonds_max: 4 } });

    expect(await store.setEnabled(false)).toBe(true);
    expect(store.getState().bluetooth).toMatchObject({ enabled: false, state: 'off', bonds: [] });
    expect(fake.bluetooth.state('mock0001').bonds).toHaveLength(2);

    expect(await store.setEnabled(true)).toBe(true);
    expect(store.getState().bluetooth).toMatchObject({ enabled: true, state: 'on', bonds: DEMO_BONDS });
    expect(store.getState().busy).toBeNull();
  });

  test('opens the pairing window, sees the code go up on the Device, and a new bond closes it', async () => {
    const store = keyboardFor(desk, 5);
    await store.load();
    expect(await store.openPairing()).toBe(true);
    expect(store.getState().bluetooth).toMatchObject({ pairing: true, passkey: false, advertising: 'pairing' });
    expect(store.getState().bluetooth?.pairing_ms).toBeGreaterThan(110_000);

    fake.bluetooth.showPasskey('mock0001', true);
    await vi.waitFor(() => expect(store.getState().bluetooth?.passkey).toBe(true));
    // Only whether a code is up reaches the app, never the code.
    expect(Object.keys(store.getState().bluetooth ?? {})).not.toContain('code');

    expect(fake.bluetooth.hostPairs('mock0001', NEW_HOST)).toBe(true);
    await vi.waitFor(() => expect(store.getState().bluetooth?.pairing).toBe(false));
    expect(store.getState().bluetooth?.bonds.map((b) => b.address)).toEqual([...DEMO_BONDS.map((b) => b.address), NEW_HOST]);
    expect(store.getState().bluetooth?.bonds.find((b) => b.address === NEW_HOST)?.connected).toBe(true);
  });

  test('stops the pairing window', async () => {
    const store = keyboardFor(desk);
    await store.load();
    await store.openPairing();
    expect(await store.closePairing()).toBe(true);
    expect(store.getState().bluetooth).toMatchObject({ pairing: false, pairing_ms: 0 });
  });

  test('forgets one computer or phone, then all of them', async () => {
    const store = keyboardFor(desk);
    await store.load();
    expect(await store.forget(DEMO_BONDS[0].address)).toBe(true);
    expect(store.getState().bluetooth?.bonds).toEqual([DEMO_BONDS[1]]);
    expect(await store.forgetAll()).toBe(true);
    expect(store.getState().bluetooth?.bonds).toEqual([]);
    expect(fake.bluetooth.state('mock0001').bonds).toEqual([]);
    expect(store.getState().forgetting).toBeNull();
  });

  test('a full Device refuses a new window and says how many it keeps', async () => {
    const full = ['01', '02', '03', '04'].map((n) => ({ address: `00:00:5e:00:53:${n}`, random: false, connected: false }));
    start({ mock0001: { enabled: true, bonds: full } });
    const store = keyboardFor(desk);
    await store.load();
    expect(await store.openPairing()).toBe(false);
    expect(store.getState().error).toBe('Desk keeps 4 computers and phones. Forget one to pair another.');
    expect(store.getState().bluetooth).toMatchObject({ pairing: false, bonds: full });
  });

  test('a Device that is off refuses the window, and the refusal brings its state', async () => {
    start({});
    const store = keyboardFor(desk);
    await store.load();
    expect(store.getState().bluetooth).toMatchObject({ enabled: false, state: 'off' });
    expect(await store.openPairing()).toBe(false);
    expect(store.getState().error).toBe('Turn on the Bluetooth keyboard first.');
  });

  test('a radio that will not start leaves the setting on and says so', async () => {
    start({});
    const store = keyboardFor(desk);
    await store.load();
    fake.bluetooth.failStart(true);
    expect(await store.setEnabled(true)).toBe(false);
    expect(store.getState().bluetooth).toMatchObject({ enabled: true, state: 'failed' });
    expect(store.getState().error).toBe("Bluetooth didn't start on Desk. Restart Desk to try again.");
  });

  test('sends nothing to a Device linked over Wi-Fi', async () => {
    const overWifi: DeviceSummary = { ...desk, carrier: 'wifi', link: { ...USB, port: '192.0.2.21:47820' } };
    const store = keyboardFor(overWifi);
    await store.load();
    expect(await store.setEnabled(false)).toBe(false);
    expect(await store.openPairing()).toBe(false);
    expect(await store.forgetAll()).toBe(false);
    expect(bluetoothCalls()).toBe(0);
    expect(fake.bluetooth.state('mock0001')).toMatchObject({ enabled: true, bonds: DEMO_BONDS });
  });

  test("the Companion's USB-only refusal asks for a cable", async () => {
    const store = keyboardFor(desk);
    await store.load();
    fake.update('mock0001', { carrier: 'wifi' });
    expect(await store.setEnabled(false)).toBe(false);
    expect(store.getState().error).toBe('Plug in Desk with a USB cable to change its Bluetooth keyboard.');
    expect(fake.bluetooth.state('mock0001').enabled).toBe(true);
  });

  test('a Device without the Bluetooth keyboard asks nothing, and older Firmware that answers needs Firmware', async () => {
    const plain = keyboardFor(studio);
    await plain.load();
    expect(plain.getState()).toMatchObject({ load: 'idle', bluetooth: null });
    expect(bluetoothCalls()).toBe(0);

    const claimed = keyboardFor({ ...studio, capabilities: RADIO_CAPS });
    await claimed.load();
    expect(claimed.getState()).toMatchObject({ needs: true, error: null });
  });

  test('polls while starting and stops once on, and pause stops it', async () => {
    fake = createFakeCompanion({ devices: [desk, studio], bluetooth: { startMs: 40 } });
    vi.stubGlobal('fetch', vi.fn(fake.fetch));
    companion = createClient(BASE, windowSession(fake.bridge));
    const store = keyboardFor(desk, 5);
    await store.load();
    expect(await store.setEnabled(true)).toBe(true);
    expect(store.getState().bluetooth?.state).toBe('starting');
    await vi.waitFor(() => expect(store.getState().bluetooth?.state).toBe('on'));

    await store.openPairing();
    store.pause();
    const before = bluetoothCalls();
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(bluetoothCalls()).toBe(before);
  });
});
