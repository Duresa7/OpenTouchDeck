import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import type { DeviceSettings, DeviceSummary, SettingsPatched } from '../companion/api.ts';
import { createClient, type Companion } from '../companion/client.ts';
import { ApiError } from '../companion/errors.ts';
import { createFakeCompanion, type FakeCompanion } from '../companion/fake.ts';
import { sampleDevice } from '../companion/samples.ts';
import { windowSession } from '../companion/session.ts';
import { createDeviceSettings, nameProblem, type DeviceSettingsDeps, type DeviceSettingsStore } from './settings.ts';

const BASE = 'http://127.0.0.1:51234';
const connected = { state: 'connected' as const, port: 'COM3', since_ms: 0, protocol: 2 };
const desk = sampleDevice('mock0001', 'Desk', connected);
const old = { ...sampleDevice('mock0002', 'Old', connected), firmware: { version: '1.2.0', protocol: 1, build: 'fake', recovery: false, update_available: null, channel: 'normal' as const } };

let fake: FakeCompanion;
let companion: Companion;
const opened: DeviceSettingsStore[] = [];

function settingsFor(device: DeviceSummary, overrides: Partial<DeviceSettingsDeps> = {}): DeviceSettingsStore {
  const store = createDeviceSettings({ companion, device, ...overrides });
  opened.push(store);
  return store;
}

async function loaded(device: DeviceSummary, overrides: Partial<DeviceSettingsDeps> = {}) {
  const store = settingsFor(device, overrides);
  await store.load();
  return store;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

const patched = (settings: DeviceSettings, restart_required = false): SettingsPatched => ({ settings, restart_required });

function stored(store: DeviceSettingsStore): DeviceSettings {
  const saved = store.getState().saved;
  if (!saved) throw new Error('The settings were never read.');
  return saved;
}

beforeEach(() => {
  fake = createFakeCompanion({ devices: [desk, old], brightness: { mock0001: 80 }, photoFrame: ['mock0001'] });
  vi.stubGlobal('fetch', fake.fetch);
  companion = createClient(BASE, windowSession(fake.bridge));
});

afterEach(() => {
  for (const store of opened.splice(0)) store.dispose();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('the name rule', () => {
  test.each([
    ['', 'Give the Device a name.'],
    ['   ', 'Give the Device a name.'],
    ['a'.repeat(49), 'That name is too long for the Device.'],
    ['é'.repeat(25), 'That name is too long for the Device.'],
    ['Desk\tleft', 'Remove tabs, line breaks and other special characters.'],
    ['Desk\u007f', 'Remove tabs, line breaks and other special characters.'],
  ])('%j is refused', (name, problem) => {
    expect(nameProblem(name)).toBe(problem);
  });

  test.each(['Desk', 'a'.repeat(48), 'é'.repeat(24), 'Stream deck 🎛'])('%j passes', (name) => {
    expect(nameProblem(name)).toBeNull();
  });
});

describe('loading', () => {
  test('reads the settings and notes what the Firmware lacks', async () => {
    const store = await loaded(desk);
    expect(store.getState().load).toBe('ready');
    expect(store.getState().shown).toEqual({
      idle_timeout_s: 600,
      orientation: 'landscape',
      name: 'Desk',
      brightness: 80,
      photo_frame: { enabled: false, timeout_s: 60, interval_s: 15, folder: null },
    });
    expect(store.getState().needs).toEqual({});
  });

  test('v1 Firmware can change neither the name, the brightness nor the photo frame', async () => {
    const store = await loaded(old);
    expect(store.getState().needs).toEqual({ name: { since: null }, brightness: { since: null }, photo_frame: { since: null } });
  });

  test('a failed read says so', async () => {
    vi.spyOn(companion.devices, 'settings_get').mockRejectedValue(new ApiError('device.not_connected', 'The Device is not connected.'));
    const store = await loaded(desk);
    expect(store.getState().load).toBe('failed');
    expect(store.getState().notice).toMatchObject({ kind: 'error', event: 'read-failed', text: "Couldn't read Desk's settings. Unplug it and plug it back in." });
  });
});

describe('renaming', () => {
  test('shows the new name at once, then keeps what the Device stored', async () => {
    const store = await loaded(desk);
    const saving = store.rename('Office');
    expect(store.getState().shown?.name).toBe('Office');
    expect(await saving).toBe(true);
    expect(store.getState().saved?.name).toBe('Office');
    expect(store.getState().notice).toMatchObject({ kind: 'ok', event: 'saved', text: 'Saved' });
    expect((await companion.devices.get({ device: 'mock0001' })).name).toBe('Office');
  });

  test('goes back to the stored name when the save fails', async () => {
    const store = await loaded(desk);
    vi.spyOn(companion.devices, 'settings_patch').mockRejectedValue(new ApiError('device.not_connected', 'The Device is not connected.'));
    const saving = store.rename('Office');
    expect(store.getState().shown?.name).toBe('Office');
    expect(await saving).toBe(false);
    expect(store.getState().shown?.name).toBe('Desk');
    expect(store.getState().notice).toEqual({
      kind: 'error',
      event: 'save-failed',
      text: "Couldn't save to Desk. Check that it's plugged in, then retry.",
      detail: 'The Device is not connected.',
    });
  });

  test('refuses a blank name without asking the Device', async () => {
    const store = await loaded(desk);
    const patch = vi.spyOn(companion.devices, 'settings_patch');
    expect(await store.rename('  ')).toBe(false);
    expect(store.getState().problem.name).toBe('Give the Device a name.');
    expect(patch).not.toHaveBeenCalled();
    expect(await store.rename('Office')).toBe(true);
    expect(store.getState().problem.name).toBeUndefined();
  });

  test('on v1 Firmware it sends nothing', async () => {
    const store = await loaded(old);
    const patch = vi.spyOn(companion.devices, 'settings_patch');
    expect(await store.rename('New')).toBe(false);
    expect(patch).not.toHaveBeenCalled();
  });

  test('needs_firmware from the Device rolls back and names the version, with no error', async () => {
    const store = await loaded(desk);
    vi.spyOn(companion.devices, 'settings_patch').mockRejectedValue(
      new ApiError('device.needs_firmware', 'This needs newer Firmware on the Device.', { field: 'name', version: '1.6.0' }),
    );
    expect(await store.rename('Office')).toBe(false);
    expect(store.getState().shown?.name).toBe('Desk');
    expect(store.getState().needs.name).toEqual({ since: '1.6.0' });
    expect(store.getState().notice).toBeNull();
  });

  test('restart_required on a name asks for a restart', async () => {
    const store = await loaded(desk);
    const settings = stored(store);
    vi.spyOn(companion.devices, 'settings_patch').mockResolvedValue(patched({ ...settings, name: 'Office' }, true));
    await store.rename('Office');
    expect(store.getState().notice).toMatchObject({ kind: 'info', event: 'restart-required', text: 'Saved. Restart Desk to apply it.' });
    expect(store.getState().restart).toBe('idle');
  });

  test('an older reply does not overwrite a newer name still on its way', async () => {
    const store = await loaded(desk);
    const settings = stored(store);
    const first = deferred<SettingsPatched>();
    const second = deferred<SettingsPatched>();
    vi.spyOn(companion.devices, 'settings_patch').mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const one = store.rename('One');
    const two = store.rename('Two');
    first.resolve(patched({ ...settings, name: 'One' }));
    await one;
    expect(store.getState().shown?.name).toBe('Two');
    second.resolve(patched({ ...settings, name: 'Two' }));
    await two;
    expect(store.getState().shown?.name).toBe('Two');
  });
});

describe('orientation', () => {
  test('asks first, and choosing the current one asks nothing', async () => {
    const store = await loaded(desk);
    store.chooseOrientation('portrait');
    expect(store.getState().confirm).toBe('portrait');
    store.chooseOrientation('landscape');
    expect(store.getState().confirm).toBeNull();
    store.chooseOrientation('portrait');
    store.cancelOrientation();
    expect(store.getState().confirm).toBeNull();
    expect(store.getState().shown?.orientation).toBe('landscape');
  });

  test('confirming saves it and restarts the Device, which drops and comes back', async () => {
    const store = await loaded(desk);
    const load = vi.spyOn(companion.devices, 'settings_get');
    store.chooseOrientation('portrait');
    await store.confirmOrientation();
    expect(store.getState().shown?.orientation).toBe('portrait');
    expect(store.getState().restart).toBe('restarting');
    expect(store.getState().notice).toMatchObject({ kind: 'info', event: 'restarting', text: 'Restarting…' });
    store.observe({ ...desk, link: { state: 'connecting', port: 'COM3' } });
    expect(store.getState().restart).toBe('restarting');
    store.observe(desk);
    expect(store.getState().restart).toBe('idle');
    expect(store.getState().notice).toBeNull();
    expect(load).toHaveBeenCalledTimes(1);
    expect(await (await fetch(`${BASE}/api/v1/test/devices/mock0001/record`)).json()).toMatchObject({ settings: { orientation: 'portrait' }, reboots: 1 });
  });

  test('a failed save keeps the old orientation and does not restart', async () => {
    const store = await loaded(desk);
    vi.spyOn(companion.devices, 'settings_patch').mockRejectedValue(new ApiError('internal', 'Boom.'));
    const restart = vi.spyOn(companion.devices, 'restart');
    store.chooseOrientation('portrait');
    await store.confirmOrientation();
    expect(store.getState().shown?.orientation).toBe('landscape');
    expect(store.getState().notice?.event).toBe('save-failed');
    expect(restart).not.toHaveBeenCalled();
  });
});

describe('restarting', () => {
  test('a refused restart says so', async () => {
    const store = await loaded(desk);
    vi.spyOn(companion.devices, 'restart').mockRejectedValue(new ApiError('device.not_connected', 'The Device is not connected.'));
    await store.restart();
    expect(store.getState().restart).toBe('failed');
    expect(store.getState().notice).toMatchObject({ kind: 'error', event: 'restart-failed', text: "Couldn't restart Desk. Unplug it and plug it back in." });
  });

  test('a Device that drops and never comes back fails', async () => {
    vi.useFakeTimers();
    const store = await loaded(desk, { restartTimeoutMs: 1000 });
    await store.restart();
    store.observe({ ...desk, link: { state: 'offline' } });
    await vi.advanceTimersByTimeAsync(1000);
    expect(store.getState().restart).toBe('failed');
    expect(store.getState().notice?.event).toBe('restart-failed');
  });

  test('a Device that never drops is taken as restarted when the wait ends', async () => {
    vi.useFakeTimers();
    const store = await loaded(desk, { restartTimeoutMs: 1000 });
    await store.restart();
    await vi.advanceTimersByTimeAsync(1000);
    expect(store.getState().restart).toBe('idle');
    expect(store.getState().notice).toBeNull();
  });
});

describe('brightness', () => {
  test('sends at most one change at a time, and the latest wins', async () => {
    vi.useFakeTimers();
    const store = await loaded(desk, { throttleMs: 150 });
    const settings = stored(store);
    const replies = [deferred<SettingsPatched>(), deferred<SettingsPatched>()];
    const patch = vi
      .spyOn(companion.devices, 'settings_patch')
      .mockReturnValueOnce(replies[0].promise)
      .mockReturnValueOnce(replies[1].promise);
    store.slideBrightness(70);
    store.slideBrightness(60);
    store.slideBrightness(50);
    expect(patch).toHaveBeenCalledTimes(1);
    expect(patch.mock.calls[0][2].brightness).toBe(70);
    expect(store.getState().shown?.brightness).toBe(50);
    replies[0].resolve(patched({ ...settings, brightness: 70 }));
    await vi.advanceTimersByTimeAsync(0);
    expect(store.getState().shown?.brightness).toBe(50);
    expect(patch).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(150);
    expect(patch).toHaveBeenCalledTimes(2);
    expect(patch.mock.calls[1][2].brightness).toBe(50);
    store.commitBrightness(50);
    replies[1].resolve(patched({ ...settings, brightness: 50 }));
    await vi.advanceTimersByTimeAsync(150);
    expect(patch).toHaveBeenCalledTimes(2);
    expect(store.getState().saved?.brightness).toBe(50);
    expect(store.getState().shown?.brightness).toBe(50);
    expect(store.getState().notice).toBeNull();
  });

  test('goes out as the percentage the slider shows, and the Companion refuses one above 100 before checking the Firmware', async () => {
    const store = await loaded(desk);
    const patch = vi.spyOn(companion.devices, 'settings_patch');
    store.slideBrightness(100);
    await vi.waitFor(() => expect(store.getState().saved?.brightness).toBe(100));
    expect(patch.mock.calls[0][2].brightness).toBe(100);
    for (const device of [desk.id, old.id]) {
      const over = companion.devices.settings_patch({ device }, null, { idle_timeout_s: null, orientation: null, name: null, brightness: 101, photo_frame: null });
      await expect(over).rejects.toMatchObject({ code: 'request.invalid' });
    }
  });

  test('a failed change goes back to the stored brightness', async () => {
    const store = await loaded(desk);
    vi.spyOn(companion.devices, 'settings_patch').mockRejectedValue(new ApiError('device.not_connected', 'The Device is not connected.'));
    store.slideBrightness(30);
    await vi.waitFor(() => expect(store.getState().notice?.event).toBe('save-failed'));
    expect(store.getState().shown?.brightness).toBe(80);
  });

  test('Firmware without brightness answers needs_firmware', async () => {
    const store = await loaded(old);
    store.slideBrightness(30);
    await vi.waitFor(() => expect(store.getState().shown?.brightness).toBeNull());
    expect(store.getState().needs.brightness).toEqual({ since: null });
    expect(store.getState().notice).toBeNull();
  });
});

describe('the photo frame', () => {
  const folder = 'D:\\Pictures\\Frame';

  test('starts off, and Firmware without it needs newer Firmware', async () => {
    expect((await loaded(desk)).getState().shown?.photo_frame).toEqual({ enabled: false, timeout_s: 60, interval_s: 15, folder: null });
    const store = await loaded(old);
    expect(store.getState().needs.photo_frame).toEqual({ since: null });
    const patch = vi.spyOn(companion.devices, 'settings_patch');
    expect(await store.setPhotoFrame({ enabled: true })).toBe(false);
    expect(patch).not.toHaveBeenCalled();
  });

  test('sends only what changed and shows it at once', async () => {
    const store = await loaded(desk);
    const patch = vi.spyOn(companion.devices, 'settings_patch');
    const turning = store.setPhotoFrame({ enabled: true });
    expect(store.getState().shown?.photo_frame?.enabled).toBe(true);
    expect(await turning).toBe(true);
    expect(patch.mock.calls[0][2].photo_frame).toEqual({ enabled: true, timeout_s: null, interval_s: null, folder: null });
    await store.setPhotoFrame({ timeout_s: 300 });
    await store.setPhotoFrame({ folder });
    expect(patch.mock.calls[2][2].photo_frame).toEqual({ enabled: null, timeout_s: null, interval_s: null, folder });
    expect(stored(store).photo_frame).toEqual({ enabled: true, timeout_s: 300, interval_s: 15, folder });
    expect(store.getState().notice?.event).toBe('saved');
  });

  test('a folder the Companion refuses goes back and asks for another, with no error notice', async () => {
    const store = await loaded(desk);
    await store.setPhotoFrame({ enabled: true, folder });
    vi.spyOn(companion.devices, 'settings_patch').mockRejectedValue(
      new ApiError('request.invalid', 'The request body is invalid.', { part: 'body', reason: 'photo_frame.folder is not the full path of a folder on this computer' }),
    );
    expect(await store.setPhotoFrame({ folder: 'E:\\Gone' })).toBe(false);
    expect(store.getState().shown?.photo_frame?.folder).toBe(folder);
    expect(store.getState().problem.folder).toBe("That folder isn't on this PC anymore. Choose another.");
    expect(store.getState().notice).toBeNull();
    vi.restoreAllMocks();
    await store.setPhotoFrame({ folder: 'E:\\Frame' });
    expect(store.getState().problem.folder).toBeUndefined();
  });

  test('a failed save goes back and says so', async () => {
    const store = await loaded(desk);
    vi.spyOn(companion.devices, 'settings_patch').mockRejectedValue(new ApiError('device.not_connected', 'The Device is not connected.'));
    await store.setPhotoFrame({ enabled: true });
    expect(store.getState().shown?.photo_frame?.enabled).toBe(false);
    expect(store.getState().notice?.event).toBe('save-failed');
  });
});
