import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import type { DeviceSummary, LinkState } from '../companion/api.ts';
import { createClient, type Companion } from '../companion/client.ts';
import { createDevicesStore, type DevicesStore } from '../companion/devices.ts';
import { openEvents, type Events } from '../companion/events.ts';
import { createFakeCompanion, type FakeCompanion } from '../companion/fake.ts';
import { freshDevice } from '../companion/fake.ts';
import { createJobsStore, waitForJob, type JobsStore } from '../companion/jobs.ts';
import { sampleDevice } from '../companion/samples.ts';
import { windowSession } from '../companion/session.ts';
import { isSent, readDraft, writeDraft, type DraftStorage } from '../editor/store.ts';
import { canonical, newLayout, starterLayout, type Orientation } from '../layout/model.ts';
import { createSetup, describeOnDevice, flowStep, nameError, startFlow, stepCount, type Flow, type SetupStore } from './onboarding.ts';
import { createDeviceSettings, type DeviceSettingsStore } from './settings.ts';

const BASE = 'http://127.0.0.1:51234';
const URL = 'ws://127.0.0.1:51234/api/v1/events';

const walk = (flow: Flow, ...moves: ('next' | 'back')[]) => moves.reduce((at, type) => flowStep(at, { type }), flow);

describe('the steps', () => {
  test('a Device on current Firmware names, stands, picks a Layout and is done', () => {
    const flow = startFlow({ firmware: false, firmwareFirst: false });
    expect(flow).toEqual({ steps: ['name', 'orientation', 'layout', 'done'], at: 'name' });
    expect(walk(flow, 'next').at).toBe('orientation');
    expect(walk(flow, 'next', 'next').at).toBe('layout');
    expect(walk(flow, 'next', 'next', 'next').at).toBe('done');
  });

  test('back returns a step, and neither end moves past itself', () => {
    const flow = startFlow({ firmware: false, firmwareFirst: false });
    expect(walk(flow, 'next', 'next', 'back').at).toBe('orientation');
    expect(walk(flow, 'back').at).toBe('name');
    expect(walk(flow, 'next', 'next', 'next', 'next').at).toBe('done');
  });

  test('a newer Firmware adds its step after the Layout, and Skip moves past it', () => {
    const flow = startFlow({ firmware: true, firmwareFirst: false });
    expect(flow.steps).toEqual(['name', 'orientation', 'layout', 'firmware', 'done']);
    const firmware = walk(flow, 'next', 'next', 'next');
    expect(firmware.at).toBe('firmware');
    expect(walk(firmware, 'next').at).toBe('done');
  });

  test('a v1 Device updates first, since naming needs newer Firmware', () => {
    const flow = startFlow({ firmware: true, firmwareFirst: true });
    expect(flow).toEqual({ steps: ['firmware', 'name', 'orientation', 'layout', 'done'], at: 'firmware' });
    expect(walk(flow, 'next').at).toBe('name');
  });

  test('an offer that arrives on the first step goes first for a v1 Device', () => {
    const flow = flowStep(startFlow({ firmware: false, firmwareFirst: true }), { type: 'offer', firmwareFirst: true });
    expect(flow).toEqual({ steps: ['firmware', 'name', 'orientation', 'layout', 'done'], at: 'firmware' });
  });

  test('a later offer adds the step after the Layout and keeps the place', () => {
    const later = walk(startFlow({ firmware: false, firmwareFirst: true }), 'next');
    expect(flowStep(later, { type: 'offer', firmwareFirst: true })).toEqual({ steps: ['name', 'orientation', 'layout', 'firmware', 'done'], at: 'orientation' });
    const current = startFlow({ firmware: false, firmwareFirst: false });
    expect(flowStep(current, { type: 'offer', firmwareFirst: false })).toEqual({ steps: ['name', 'orientation', 'layout', 'firmware', 'done'], at: 'name' });
  });

  test('a second offer changes nothing', () => {
    const flow = startFlow({ firmware: true, firmwareFirst: false });
    expect(flowStep(flow, { type: 'offer', firmwareFirst: true })).toBe(flow);
  });

  test('a Device on the radio build gets the Wi-Fi step last, before done', () => {
    expect(startFlow({ firmware: false, firmwareFirst: false, wifi: true }).steps).toEqual(['name', 'orientation', 'layout', 'wifi', 'done']);
    expect(startFlow({ firmware: true, firmwareFirst: false, wifi: true }).steps).toEqual(['name', 'orientation', 'layout', 'firmware', 'wifi', 'done']);
    const offered = flowStep(startFlow({ firmware: false, firmwareFirst: true, wifi: true }), { type: 'offer', firmwareFirst: true });
    expect(offered).toEqual({ steps: ['firmware', 'name', 'orientation', 'layout', 'wifi', 'done'], at: 'firmware' });
    expect(startFlow({ firmware: true, firmwareFirst: false }).steps).not.toContain('wifi');
  });

  test('the count follows the place', () => {
    const flow = startFlow({ firmware: true, firmwareFirst: false });
    expect(stepCount(flow)).toEqual({ n: 1, total: 5 });
    expect(stepCount(walk(flow, 'next', 'next', 'next', 'next'))).toEqual({ n: 5, total: 5 });
  });
});

describe('names', () => {
  test.each([
    ['Desk', null],
    ['a'.repeat(48), null],
    ['', 'Enter a name.'],
    ['   ', 'Enter a name.'],
    ['a'.repeat(49), 'Names can be up to 48 characters.'],
    ['é'.repeat(25), 'Names can be up to 48 characters.'],
    ['Desk\tTwo', 'Remove tabs, line breaks and other special characters.'],
  ])('%j reads %j', (name, error) => {
    expect(nameError(name)).toBe(error);
  });
});

test('the Layout on the Device is told in Pages and Tiles', () => {
  expect(describeOnDevice(newLayout('landscape'))).toBe('The Layout on the Device now: 1 Page, 0 Tiles.');
});

function memoryStorage(): DraftStorage {
  const items = new Map<string, string>();
  return {
    getItem: (key) => items.get(key) ?? null,
    setItem: (key, value) => void items.set(key, value),
    removeItem: (key) => void items.delete(key),
  };
}

describe('setting up a Device', () => {
  const connected = { state: 'connected', port: 'COM3', since_ms: 0, protocol: 2 } satisfies LinkState;
  const shelf = freshDevice({ id: 'mock0009', port: 'COM9', firmware: { version: '1.5.0', protocol: 2 } });
  const old = freshDevice({ id: 'mock0008', port: 'COM8', firmware: { version: '1.2.0', protocol: 1 } });

  let fake: FakeCompanion;
  let companion: Companion;
  let jobs: JobsStore;
  let devices: DevicesStore;
  let events: Events;
  let drafts: DraftStorage;
  const opened: { settings: DeviceSettingsStore; stop(): void }[] = [];

  function start(fresh: FakeCompanion) {
    fake = fresh;
    vi.stubGlobal('fetch', fake.fetch);
    companion = createClient(BASE, windowSession(fake.bridge));
    jobs = createJobsStore();
    devices = createDevicesStore();
    events = openEvents({ url: URL, session: windowSession(fake.bridge), store: devices, jobs, open: fake.openSocket });
  }

  function setup(device: DeviceSummary, offered: string | null = null): SetupStore {
    const settings = createDeviceSettings({ companion, device });
    const store = createSetup({ companion, wait: (job) => waitForJob<'layouts.push'>(jobs, companion, job, { pollMs: 10 }), settings, drafts, device, firmware: { offered } });
    const stop = devices.subscribe((state) => {
      const latest = state.devices.find((d) => d.id === device.id);
      if (!latest) return;
      settings.observe(latest);
      store.observe(latest);
    });
    opened.push({ settings, stop });
    return store;
  }

  const record = async (id: string) => (await fetch(`${BASE}/api/v1/test/devices/${id}/record`)).json();
  const pushes = () => fake.layouts.puts;
  const stored = (id: string, orientation: Orientation) => fake.layouts.stored(id, orientation);

  async function atLayout(store: SetupStore) {
    store.setName('Shelf');
    await store.submitName();
    await vi.waitFor(() => expect(devices.getState().devices.find((d) => d.id === 'mock0009')?.name).toBe('Shelf'));
    await store.submitOrientation();
    await vi.waitFor(() => expect(store.getState().busy).toBeNull());
    expect(store.getState().flow.at).toBe('layout');
  }

  beforeEach(() => {
    drafts = memoryStorage();
    start(createFakeCompanion({ devices: [sampleDevice('mock0001', 'Desk', connected), shelf, old], jobMs: 2 }));
  });

  afterEach(() => {
    for (const { settings, stop } of opened.splice(0)) {
      stop();
      settings.dispose();
    }
    events.close();
    vi.unstubAllGlobals();
  });

  test('only a Device on the radio build is offered Wi-Fi setup, after its other steps', () => {
    const radio: DeviceSummary = { ...shelf, firmware: shelf.firmware && { ...shelf.firmware, channel: 'radio' } };
    expect(setup(radio).getState().flow.steps).toEqual(['name', 'orientation', 'layout', 'wifi', 'done']);
    expect(setup(shelf).getState().flow.steps).not.toContain('wifi');
  });

  test('a name saves to the Device and marks it set up', async () => {
    const store = setup(shelf);
    store.setName('  Shelf  ');
    await store.submitName();
    expect(store.getState()).toMatchObject({ name: 'Shelf', nameError: null, busy: null, flow: { at: 'orientation' } });
    expect(await companion.devices.get({ device: 'mock0009' })).toMatchObject({ name: 'Shelf', setup: 'ready' });
  });

  test('a name the Device would refuse stays on the step with the reason', async () => {
    const store = setup(shelf);
    store.setName('a'.repeat(49));
    await store.submitName();
    expect(store.getState()).toMatchObject({ nameError: 'Names can be up to 48 characters.', flow: { at: 'name' } });
    store.setName('Shelf');
    expect(store.getState().nameError).toBeNull();
    expect((await companion.devices.get({ device: 'mock0009' })).setup).toBe('needs-setup');
  });

  test('a v1 Device says naming needs newer Firmware, then Next moves on', async () => {
    const store = setup(old);
    expect(store.getState().flow.at).toBe('name');
    store.setName('Old');
    await store.submitName();
    expect(store.getState()).toMatchObject({ needsFirmware: true, error: null, flow: { at: 'name' } });
    await store.submitName();
    expect(store.getState().flow.at).toBe('orientation');
  });

  test('a v1 Device with newer Firmware on offer starts on the Firmware step', () => {
    expect(setup(old, '1.5.0').getState().flow).toEqual({ steps: ['firmware', 'name', 'orientation', 'layout', 'done'], at: 'firmware' });
    expect(setup(shelf, '1.6.0').getState().flow.steps).toEqual(['name', 'orientation', 'layout', 'firmware', 'done']);
  });

  test('a save that fails stays on the step and says why', async () => {
    const store = setup(shelf);
    fake.update('mock0009', { link: { state: 'lost', reason: 'unplugged', detail: '', at_ms: 0 } });
    store.setName('Shelf');
    await store.submitName();
    expect(store.getState()).toMatchObject({ flow: { at: 'name' }, error: { text: "Couldn't save to the Device. Check that it's plugged in, then retry." } });
  });

  test('keeping the orientation moves on and reads the Layout on the Device', async () => {
    fake.layouts.store('mock0009', newLayout('landscape'));
    const store = setup(shelf);
    store.setName('Shelf');
    await store.submitName();
    await store.submitOrientation();
    expect(store.getState().flow.at).toBe('layout');
    expect(store.getState()).toMatchObject({ busy: null, choice: 'keep', onDevice: { layout: { name: 'New Layout' } } });
    expect((await record('mock0009')).reboots).toBe(0);
  });

  test('a new orientation saves, restarts the Device and waits for it to come back', async () => {
    const store = setup(shelf);
    store.setName('Shelf');
    await store.submitName();
    store.chooseOrientation('portrait');
    const step = store.submitOrientation();
    await vi.waitFor(() => expect(store.getState().busy).toBe('restarting'));
    await step;
    await vi.waitFor(() => expect(store.getState().busy).toBeNull());
    expect(store.getState()).toMatchObject({ orientation: 'portrait', flow: { at: 'layout' }, error: null, choice: 'starter', onDevice: null });
    expect(await record('mock0009')).toMatchObject({ reboots: 1, settings: { orientation: 'portrait' } });
  });

  test('Starter sends the Starter Layout and keeps it as a sent draft', async () => {
    const store = setup(shelf);
    await atLayout(store);
    expect(store.getState().choice).toBe('starter');
    await store.submitLayout();
    expect(store.getState().flow.at).toBe('done');
    expect(stored('mock0009', 'landscape')).toBe(canonical(starterLayout('landscape')));
    const draft = readDraft(drafts, 'mock0009', 'landscape');
    expect(draft?.layout.name).toBe('Starter');
    expect(draft && isSent(draft)).toBe(true);
  });

  test('Blank sends one empty Page', async () => {
    const store = setup(shelf);
    await atLayout(store);
    store.chooseLayout('blank');
    await store.submitLayout();
    expect(store.getState().flow.at).toBe('done');
    expect(stored('mock0009', 'landscape')).toBe(canonical(newLayout('landscape')));
    expect(readDraft(drafts, 'mock0009', 'landscape')?.layout.name).toBe('New Layout');
  });

  test('Keep leaves the Device alone and opens its Layout as the draft', async () => {
    fake.layouts.store('mock0009', newLayout('landscape'));
    const store = setup(shelf);
    await atLayout(store);
    expect(store.getState().choice).toBe('keep');
    const before = pushes();
    await store.submitLayout();
    expect(store.getState().flow.at).toBe('done');
    expect(pushes()).toBe(before);
    const draft = readDraft(drafts, 'mock0009', 'landscape');
    expect(draft?.layout.name).toBe('New Layout');
    expect(draft && isSent(draft)).toBe(true);
  });

  test('Keep never replaces a draft that is already open', async () => {
    fake.layouts.store('mock0009', newLayout('landscape'));
    const mine = { ...starterLayout('landscape'), name: 'Mine' };
    writeDraft(drafts, 'mock0009', 'landscape', { layout: mine, onDevice: null, revision: null });
    const store = setup(shelf);
    await atLayout(store);
    await store.submitLayout();
    expect(readDraft(drafts, 'mock0009', 'landscape')?.layout.name).toBe('Mine');
  });

  test('Keep is only offered when the Device has a Layout', async () => {
    const store = setup(shelf);
    await atLayout(store);
    store.chooseLayout('keep');
    expect(store.getState().choice).toBe('starter');
  });

  test('a send that fails stays on the step and says why', async () => {
    const store = setup(shelf);
    await atLayout(store);
    fake.update('mock0009', { link: { state: 'lost', reason: 'unplugged', detail: '', at_ms: 0 } });
    await store.submitLayout();
    expect(store.getState()).toMatchObject({ flow: { at: 'layout' }, busy: null, error: { text: "Couldn't send to Shelf. Check that it's plugged in, then retry." } });
  });

  test('going back to the Layout step reads the Device again', async () => {
    const store = setup(shelf);
    await atLayout(store);
    await store.submitLayout();
    store.back();
    await vi.waitFor(() => expect(store.getState().busy).toBeNull());
    expect(store.getState()).toMatchObject({ flow: { at: 'layout' }, choice: 'keep', onDevice: { layout: { name: 'Starter' } } });
  });
});
