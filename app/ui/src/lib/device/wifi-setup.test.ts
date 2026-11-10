import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import type { DeviceSummary } from '../companion/api.ts';
import { createClient, type Companion } from '../companion/client.ts';
import { createFakeCompanion, type FakeCompanion } from '../companion/fake.ts';
import { RADIO_CAPS, sampleDevice } from '../companion/samples.ts';
import { windowSession } from '../companion/session.ts';
import { createWifiSetup, offersWifiSetup, passwordProblem, ssidProblem, wifiSetupStatusText, type WifiSetupStore } from './wifi-setup.ts';

// Every network and password here is made up.
const BASE = 'http://127.0.0.1:51234';
const NETWORK = 'td-demo-network';
const PASSWORD = 'demo-pass-1234';
const connected = { state: 'connected' as const, port: 'COM3', since_ms: 0, protocol: 2 };
const radio: DeviceSummary = { ...sampleDevice('mock0001', 'Desk', connected), capabilities: RADIO_CAPS };
const normal = sampleDevice('mock0002', 'Studio', connected);

let fake: FakeCompanion;
let companion: Companion;
let sent: string[];
const opened: WifiSetupStore[] = [];

function wifiFor(device: DeviceSummary): WifiSetupStore {
  const store = createWifiSetup({ companion, device, pollMs: 10, pollForMs: 2000 });
  opened.push(store);
  return store;
}

async function settled(store: WifiSetupStore) {
  await vi.waitFor(() => expect(store.getState().wifi?.state).not.toBe('connecting'), { timeout: 2000 });
  return store.getState().wifi;
}

beforeEach(() => {
  fake = createFakeCompanion({ devices: [radio, normal], wifiSetup: { joinMs: 30 } });
  sent = [];
  vi.stubGlobal('fetch', async (input: string | URL | Request, init?: RequestInit) => {
    const request = new Request(input, init);
    if (request.method === 'PUT') sent.push(await request.clone().text());
    return fake.fetch(request);
  });
  companion = createClient(BASE, windowSession(fake.bridge));
});

afterEach(() => {
  for (const store of opened.splice(0)) store.dispose();
  vi.unstubAllGlobals();
});

describe('the network rules', () => {
  test.each([
    ['', 'Type the network name.'],
    ['x'.repeat(33), 'That network name is too long.'],
    ['é'.repeat(17), 'That network name is too long.'],
    ['line\nbreak', 'Remove tabs, line breaks and other special characters.'],
  ])('network %j is refused', (ssid, problem) => {
    expect(ssidProblem(ssid)).toBe(problem);
  });

  test.each(['', '12345678', 'a'.repeat(63), 'F0'.repeat(32), ' ~dummy pass~ '])('password %j is fine', (password) => {
    expect(passwordProblem(password)).toBeNull();
  });

  test.each(['short', 'g'.repeat(64), 'a'.repeat(65), 'tab\tin here', 'café-dummy-pass', 'dummy-pass\u007f', 'dummy-pass-\u200b'])('password %j is refused', (password) => {
    expect(passwordProblem(password)).not.toBeNull();
  });

  test('only a Device advertising Wi-Fi offers it', () => {
    expect(offersWifiSetup(radio)).toBe(true);
    expect(offersWifiSetup(normal)).toBe(false);
    expect(offersWifiSetup({ capabilities: null })).toBe(false);
  });

  test('the status reads the reason the Device reports', () => {
    const failed = { configured: true, ssid: NETWORK, state: 'failed' as const, reason: 'not-found', ip: null, rssi: null };
    expect(wifiSetupStatusText(failed, 'Desk')).toBe(`Can't find ${NETWORK}. Check the name, and that Desk is in range. Desk keeps trying.`);
    expect(wifiSetupStatusText({ ...failed, reason: 'wrong-password' }, 'Desk')).toBe(`${NETWORK} refused the password. Desk keeps trying.`);
    expect(wifiSetupStatusText({ ...failed, state: 'connected', reason: null, ip: '192.0.2.10', rssi: -52 }, 'Desk')).toBe(`Connected to ${NETWORK} at 192.0.2.10`);
  });
});

describe('the Wi-Fi store', () => {
  test('joins a network and follows the join to connected', async () => {
    const store = wifiFor(radio);
    await store.load();
    expect(store.getState().wifi?.state).toBe('off');
    expect(await store.join(NETWORK, PASSWORD)).toBe(true);
    expect(store.getState().wifi?.state).toBe('connecting');
    const wifi = await settled(store);
    expect(wifi).toMatchObject({ state: 'connected', ssid: NETWORK, ip: '192.0.2.10' });
    expect(JSON.parse(sent[0])).toEqual({ ssid: NETWORK, password: PASSWORD });
  });

  test('never keeps the password', async () => {
    const store = wifiFor(radio);
    await store.load();
    await store.join(NETWORK, PASSWORD);
    await settled(store);
    expect(JSON.stringify(store.getState())).not.toContain(PASSWORD);
  });

  test('reports a network out of range and a wrong password', async () => {
    const store = wifiFor(radio);
    await store.load();
    await store.join('td-test-nonexistent', PASSWORD);
    expect(await settled(store)).toMatchObject({ state: 'failed', reason: 'not-found' });
    await store.join(NETWORK, 'wrong-pass-5678');
    expect(await settled(store)).toMatchObject({ state: 'failed', reason: 'wrong-password' });
  });

  test('joins an open network without a password', async () => {
    const store = wifiFor(radio);
    await store.load();
    await store.join('td-demo-open', '');
    expect(await settled(store)).toMatchObject({ state: 'connected', ssid: 'td-demo-open' });
  });

  test('refuses a bad network or password without sending it', async () => {
    const store = wifiFor(radio);
    await store.load();
    expect(await store.join('', PASSWORD)).toBe(false);
    expect(await store.join(NETWORK, 'short')).toBe(false);
    expect(store.getState().problem).toEqual({ password: 'A Wi-Fi password has at least 8 characters.' });
    expect(sent).toEqual([]);
  });

  test('a field that opens again shows no problem from the last drafts', async () => {
    const store = wifiFor(radio);
    await store.load();
    expect(await store.join(NETWORK, 'short')).toBe(false);
    expect(store.getState().problem.password).toBeDefined();
    await store.load();
    expect(store.getState().problem).toEqual({});
  });

  test('forgets the network', async () => {
    const store = wifiFor(radio);
    await store.load();
    await store.join(NETWORK, PASSWORD);
    await settled(store);
    expect(await store.forget()).toBe(true);
    expect(store.getState().wifi).toMatchObject({ configured: false, state: 'off', ssid: '' });
  });

  test('marks Firmware without a Wi-Fi station', async () => {
    const store = wifiFor({ ...normal, capabilities: RADIO_CAPS });
    await store.load();
    expect(store.getState().needs).toBe(true);
    expect(store.getState().error).toBeNull();
  });

  test('sends nothing while the Device is on Wi-Fi', async () => {
    const store = wifiFor({ ...radio, carrier: 'wifi' });
    await store.load();
    expect(await store.join(NETWORK, PASSWORD)).toBe(false);
    expect(store.getState().load).toBe('idle');
    expect(sent).toEqual([]);
  });

  test('reads nothing from a Device that does not offer Wi-Fi', async () => {
    const store = wifiFor(normal);
    await store.load();
    expect(store.getState().load).toBe('idle');
  });
});
