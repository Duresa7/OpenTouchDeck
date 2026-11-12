import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import type { DeviceSummary } from '../companion/api.ts';
import { createClient, type Companion } from '../companion/client.ts';
import { ApiError } from '../companion/errors.ts';
import { createFakeCompanion, FAKE_WIFI_CODE, type FakeCompanion } from '../companion/fake.ts';
import { sampleDevice } from '../companion/samples.ts';
import { windowSession } from '../companion/session.ts';
import { carrierText, createWifiPairing, physicalAccess, wifiProblem, type WifiStore } from './wifi.ts';

const BASE = 'http://127.0.0.1:51234';
const USB = { state: 'connected' as const, port: 'COM3', since_ms: 0, protocol: 2 };
const desk = sampleDevice('mock0001', 'Desk', USB);
const shelf = sampleDevice('mock0002', 'Shelf');

let fake: FakeCompanion;
let companion: Companion;

const pairingFor = (device: DeviceSummary): WifiStore => createWifiPairing({ companion, device: () => device });

async function atCode(store: WifiStore, address: string) {
  await store.getState().load();
  await store.getState().begin();
  store.getState().type(address);
  await store.getState().start();
  expect(store.getState().view.step).toBe('code');
}

beforeEach(() => {
  fake = createFakeCompanion({
    devices: [desk, shelf],
    wifi: {
      discovered: [
        { device: 'mock0001', name: 'Desk', address: '192.0.2.21', paired: 0 },
        { device: 'mock0002', name: 'Shelf', address: '192.0.2.22:47820', paired: 0 },
        { device: 'mock0009', name: 'Other', address: '192.0.2.29', paired: 0 },
      ],
    },
  });
  vi.stubGlobal('fetch', fake.fetch);
  companion = createClient(BASE, windowSession(fake.bridge));
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('the Carrier in words', () => {
  test('USB, Wi-Fi with its address, and nothing while not connected', () => {
    expect(carrierText(desk)).toBe('Connected over USB');
    expect(carrierText({ link: { ...USB, port: '192.0.2.21:47820' }, carrier: 'wifi' })).toBe('Connected over Wi-Fi at 192.0.2.21:47820');
    expect(carrierText(shelf)).toBeNull();
  });
});

describe('pairing over Wi-Fi', () => {
  test('discovery fills in the address, and the right code pairs and links the Device', async () => {
    const store = pairingFor(shelf);
    await store.getState().load();
    expect(store.getState().pairing).toBeNull();
    await store.getState().begin();
    expect(store.getState().view).toEqual({ step: 'address', found: true });
    expect(store.getState().address).toBe('192.0.2.22:47820');
    await store.getState().start();
    expect(store.getState().view).toMatchObject({ step: 'code', address: '192.0.2.22:47820', attemptsLeft: 3 });
    await store.getState().submit(FAKE_WIFI_CODE);
    expect(store.getState().view.step).toBe('idle');
    expect(store.getState().pairing).toMatchObject({ device: 'mock0002', address: '192.0.2.22:47820', confirmed: true });
    const linked = await companion.devices.get({ device: 'mock0002' });
    expect(linked).toMatchObject({ carrier: 'wifi', link: { state: 'connected', port: '192.0.2.22:47820' } });
  });

  test('a Device the browse misses takes a typed address, with the default port', async () => {
    const store = pairingFor(desk);
    await store.getState().begin();
    expect(store.getState().address).toBe('192.0.2.21:47820');
    store.getState().type('192.0.2.21');
    await store.getState().start();
    expect(store.getState().view).toMatchObject({ step: 'code', address: '192.0.2.21:47820' });
  });

  test('nothing at the address says so and keeps the address field', async () => {
    const store = pairingFor(desk);
    await store.getState().begin();
    store.getState().type('192.0.2.99');
    await store.getState().start();
    expect(store.getState().view).toEqual({ step: 'address', found: false });
    expect(store.getState().problem).toMatch(/^Nothing answered at that address/u);
  });

  test('another Device at the address is refused and its attempt cancelled', async () => {
    const store = pairingFor(desk);
    await store.getState().begin();
    store.getState().type('192.0.2.29');
    await store.getState().start();
    expect(store.getState().problem).toBe('Another Device answered at that address, not Desk.');
    expect(fake.wifi.attempts()).toEqual([]);
  });

  test('wrong codes count down, and the third one locks pairing out', async () => {
    const store = pairingFor(desk);
    await atCode(store, '192.0.2.21');
    await store.getState().submit('000000');
    expect(store.getState().view).toMatchObject({ step: 'code', attemptsLeft: 2 });
    expect(store.getState().problem).toBe('Wrong code. 2 tries left.');
    await store.getState().submit('000000');
    expect(store.getState().problem).toBe('Wrong code. 1 try left.');
    await store.getState().submit('000000');
    expect(store.getState().view.step).toBe('address');
    expect(store.getState().problem).toMatch(/^Too many wrong codes\. Desk stopped pairing/u);
  });

  test('a code that is not six digits never reaches the Companion', async () => {
    const store = pairingFor(desk);
    await atCode(store, '192.0.2.21');
    const before = fake.requests;
    await store.getState().submit('12a');
    expect(fake.requests).toBe(before);
    expect(store.getState().problem).toBe('Enter the 6 digits Desk shows.');
  });

  test('with no keychain the error says what to do, and the attempt stays open', async () => {
    fake.wifi.setKeychain(false);
    const store = pairingFor(desk);
    await atCode(store, '192.0.2.21');
    await store.getState().submit(FAKE_WIFI_CODE);
    expect(store.getState().view.step).toBe('code');
    expect(store.getState().problem).toMatch(/Credential Manager/u);
    expect(store.getState().pairing).toBeNull();
  });

  test('cancel ends the attempt on the Companion', async () => {
    const store = pairingFor(desk);
    await atCode(store, '192.0.2.21');
    expect(fake.wifi.attempts()).toHaveLength(1);
    await store.getState().cancel();
    expect(store.getState().view.step).toBe('idle');
    expect(fake.wifi.attempts()).toEqual([]);
  });

  test('forget drops the Pairing and takes the Device off Wi-Fi', async () => {
    const store = pairingFor(shelf);
    await atCode(store, '192.0.2.22');
    await store.getState().submit(FAKE_WIFI_CODE);
    await store.getState().forget();
    expect(store.getState().pairing).toBeNull();
    expect(fake.wifi.pairings()).toEqual([]);
    expect(await companion.devices.get({ device: 'mock0002' })).toMatchObject({ carrier: null, link: { state: 'offline' } });
  });

  test('a rename shows in the messages', async () => {
    let device = desk;
    const store = createWifiPairing({ companion, device: () => device });
    device = { ...desk, name: 'Desk left' };
    await atCode(store, '192.0.2.21');
    await store.getState().submit('1');
    expect(store.getState().problem).toBe('Enter the 6 digits Desk left shows.');
  });
});

describe('the words for each Wi-Fi error', () => {
  test.each([
    ['wifi.window_closed', "Desk isn't pairing. On Desk, open Settings and choose Pair new app, then try again."],
    ['wifi.expired', 'Pairing ended on Desk. Start again.'],
    ['wifi.full', 'The keychain message.'],
    ['wifi.already_paired', 'Desk still has a Pairing with this app. Forget it on Desk (paired apps screen), then pair again.'],
    ['wifi.version', "Desk's Firmware can't pair over Wi-Fi with this app. Update its Firmware over USB, then try again."],
    ['wifi.busy', 'Desk is busy with another app. Close it, then try again.'],
    ['wifi.keychain', 'The keychain message.'],
  ])('%s', (code, words) => {
    expect(wifiProblem(new ApiError(code, 'The keychain message.'), 'Desk')).toBe(words);
  });

  test('a full Device that advertised four Pairings gets the four-apps advice', () => {
    expect(wifiProblem(new ApiError('wifi.full', 'The Companion message.', { paired: 4 }), 'Desk')).toBe(
      'Four apps are paired. Forget one first: on Desk, use the paired apps screen, or plug it in with a USB cable.',
    );
  });
});

describe('a Device with four Pairings', () => {
  test('shows the advice up front and treats full the same way', async () => {
    fake = createFakeCompanion({
      devices: [desk, shelf],
      wifi: { discovered: [{ device: 'mock0002', name: 'Shelf', address: '192.0.2.22:47820', paired: 4 }] },
    });
    vi.stubGlobal('fetch', fake.fetch);
    companion = createClient(BASE, windowSession(fake.bridge));
    const store = pairingFor(shelf);
    await store.getState().load();
    await store.getState().begin();
    const advice = 'Four apps are paired. Forget one first: on Shelf, use the paired apps screen, or plug it in with a USB cable.';
    expect(store.getState().view).toEqual({ step: 'address', found: true });
    expect(store.getState().problem).toBe(advice);
    await store.getState().start();
    expect(store.getState().view).toEqual({ step: 'address', found: false });
    expect(store.getState().problem).toBe(advice);
  });

  test('the count the Device gave over USB brings the advice even when the network says less', async () => {
    const store = createWifiPairing({ companion, device: () => desk, paired: () => 4 });
    await store.getState().load();
    await store.getState().begin();
    expect(store.getState().view).toEqual({ step: 'address', found: true });
    expect(store.getState().problem).toBe('Four apps are paired. Forget one first: on Desk, use the paired apps screen, or plug it in with a USB cable.');
    const fewer = createWifiPairing({ companion, device: () => desk, paired: () => 3 });
    await fewer.getState().begin();
    expect(fewer.getState().problem).toBeNull();
  });
});

describe('the pairing notice', () => {
  test('says that the Device in hand is every Pairing on it', () => {
    expect(physicalAccess('Desk')).toBe('Whoever can get to Desk can use every Pairing on it. If Desk is lost or passed on, forget its paired apps.');
  });
});
