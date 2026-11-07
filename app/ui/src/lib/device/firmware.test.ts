import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import type { DeviceSummary, FirmwareProgress, Job } from '../companion/api.ts';
import { createClient, type Companion } from '../companion/client.ts';
import { ApiError } from '../companion/errors.ts';
import { createFakeCompanion, type FakeCompanion } from '../companion/fake.ts';
import type { FakePick } from '../companion/fake.ts';
import { createJobsStore, type JobsStore } from '../companion/jobs.ts';
import { sampleDevice } from '../companion/samples.ts';
import { windowSession } from '../companion/session.ts';
import { createFirmwareUpdate, doneText, failureText, phaseText, viewOf, type FirmwareUpdateStore, type PhaseProgress } from './firmware.ts';

const BASE = 'http://127.0.0.1:51234';
const STUDIO = { device: 'Studio', version: '1.4.0' };
const connected = { state: 'connected' as const, port: 'COM4', since_ms: 0, protocol: 2 };
const studio: DeviceSummary = {
  ...sampleDevice('mock0002', 'Studio', connected),
  firmware: { version: '1.4.0', protocol: 2, build: 'fake', recovery: false, update_available: '1.5.0', channel: 'normal' },
};

const progress = (phase: string, pct: number | null = null): FirmwareProgress => ({ phase, pct, bytes: pct === null ? 0 : pct * 15_729 });

function job(change: Partial<Job>): Job {
  return {
    id: 'job_firmware00000001',
    kind: 'firmware.update',
    scope: { kind: 'device', id: 'mock0002' },
    state: 'running',
    progress: progress('checking'),
    result: null,
    error: null,
    cancellable: true,
    created_ms: 0,
    finished_ms: null,
    rev: 1,
    ...change,
  };
}

const failed = (code: string) => job({ state: 'failed', error: { code, message: `Failed with ${code}.`, details: null }, finished_ms: 1 });

describe('phases', () => {
  test.each<[PhaseProgress | null, string]>([
    [{ phase: 'checking', pct: null }, 'Checking the file…'],
    [{ phase: 'sending', pct: 40 }, 'Updating… 40%'],
    [{ phase: 'sending', pct: 33.6 }, 'Updating… 34%'],
    [{ phase: 'recovering', pct: 25 }, 'Repairing… 25%'],
    [{ phase: 'rebooting', pct: null }, 'Restarting…'],
    [{ phase: 'confirming', pct: null }, 'Checking that it started…'],
    [null, 'Updating…'],
  ])('%j reads %j', (phase, text) => {
    expect(phaseText(phase)).toBe(text);
  });
});

describe('failures', () => {
  test.each([
    ['firmware.rolled_back', null, "The new Firmware didn't start, so Studio went back to 1.4.0. It still works. Retry, or repair it from Troubleshooting."],
    ['firmware.confirm_timeout', null, "Studio didn't come back after restarting. Unplug it and plug it back in. If its screen stays blank, repair it from Troubleshooting."],
    ['firmware.failed', { reason: 'device unplugged' }, 'Studio was unplugged during the update. Plug it back in. If its screen stays blank, repair it from Troubleshooting.'],
    ['firmware.failed', { reason: 'too many chunks' }, "That file is over 6 MB, so it isn't a Firmware file."],
    ['firmware.failed', { reason: 'write error' }, "Couldn't update. Studio still runs 1.4.0. Check the cable, then retry."],
    ['firmware.declined', null, 'Declined on this PC. Studio still runs 1.4.0.'],
    ['firmware.bundle_invalid', null, "That file isn't a Firmware file for this Device."],
    ['firmware.bundle_not_found', null, 'That file expired. Choose it again, then retry.'],
    ['firmware.channel_mismatch', { channel: 'normal', bundle: 'radio' }, 'That file is the radio build, and Studio runs the normal build. Nothing changed.'],
    ['firmware.channel_mismatch', { channel: 'radio', bundle: 'normal' }, 'That file is the normal build, and Studio runs the radio build. Nothing changed.'],
    ['device.busy', null, 'Studio is updating. Wait for it to finish.'],
    ['device.not_connected', null, "Studio isn't connected."],
    ['job.cancelled', null, 'Update stopped. Studio still runs 1.4.0.'],
    ['internal', null, "Couldn't update. Studio still runs 1.4.0. Check the cable, then retry."],
  ])('%s %j', (code, details, text) => {
    expect(failureText({ code, details }, STUDIO)).toBe(text);
  });

  test('a failed Job shows its sentence and keeps the raw message', () => {
    expect(viewOf(failed('firmware.rolled_back'), STUDIO)).toEqual({
      step: 'failed',
      text: failureText({ code: 'firmware.rolled_back', details: null }, STUDIO),
      raw: 'Failed: Failed with firmware.rolled_back.',
      code: 'firmware.rolled_back',
    });
  });

  test('a cancelled Job reads as stopped, not failed', () => {
    const cancelled = { code: 'job.cancelled', message: 'The Job was cancelled.', details: null };
    expect(viewOf(job({ state: 'cancelled', error: cancelled, finished_ms: 1 }), STUDIO)).toEqual({ step: 'cancelled', text: 'Update stopped. Studio still runs 1.4.0.', raw: 'Cancelled' });
    expect(viewOf(job({ state: 'cancelled', finished_ms: 1 }), STUDIO)).toEqual({ step: 'cancelled', text: 'Update stopped. Studio still runs 1.4.0.', raw: 'Cancelled' });
  });

  test('a consent nobody answers ends as declined, with the declined sentence', () => {
    const declined = { code: 'firmware.declined', message: 'Firmware confirmation was declined.', details: null };
    expect(viewOf(job({ state: 'failed', error: declined, finished_ms: 1 }), STUDIO)).toEqual({
      step: 'failed',
      text: 'Declined on this PC. Studio still runs 1.4.0.',
      raw: 'Failed: Firmware confirmation was declined.',
      code: 'firmware.declined',
    });
  });

  test('a failed Job without an error reads as a plain failure', () => {
    expect(viewOf(job({ state: 'failed', finished_ms: 1 }), STUDIO)).toMatchObject({ step: 'failed', code: 'internal' });
  });
});

describe('the Job view', () => {
  test('queued and waiting', () => {
    expect(viewOf(job({ state: 'queued' }), STUDIO)).toEqual({ step: 'running', text: 'Updating…', raw: 'Queued', pct: null });
    expect(viewOf(job({ state: 'waiting_for_confirmation' }), STUDIO)).toEqual({
      step: 'waiting',
      text: 'Confirm on this PC to continue.',
      raw: 'Waiting for confirmation',
    });
  });

  test('running reads the phase and the percent', () => {
    expect(viewOf(job({ progress: progress('sending', 30) }), STUDIO)).toEqual({ step: 'running', text: 'Updating… 30%', raw: 'Sending 30%', pct: 30 });
    expect(viewOf(job({ progress: progress('rebooting') }), STUDIO)).toEqual({ step: 'running', text: 'Restarting…', raw: 'Rebooting', pct: null });
  });

  test('progress it cannot read still reads as updating', () => {
    expect(viewOf(job({ progress: progress('warping') }), STUDIO)).toEqual({ step: 'running', text: 'Updating…', raw: 'Running', pct: null });
  });

  test.each([
    [{ message: 'Firmware updated', version: '1.5.0' }, 'Updated to 1.5.0'],
    [{ message: 'Already on this build', version: '1.4.0' }, 'Already on Firmware 1.4.0'],
    [{ message: 'Repaired', version: '1.5.0' }, 'Done. Repaired.'],
  ])('a result of %j reads %j', (result, text) => {
    expect(doneText(result)).toBe(text);
    expect(viewOf(job({ state: 'succeeded', result, finished_ms: 1 }), STUDIO)).toEqual({ step: 'done', text, raw: result.message, version: result.version });
  });
});

describe('the update runner', () => {
  let fake: FakeCompanion;
  let companion: Companion;
  let jobs: JobsStore;
  const runners: FirmwareUpdateStore[] = [];

  function runner(device: DeviceSummary = studio) {
    const update = createFirmwareUpdate({
      companion,
      jobs,
      context: () => ({ id: device.id, device: device.name ?? 'the Device', version: device.firmware?.version ?? '' }),
      pollMs: 5,
    });
    runners.push(update);
    return update;
  }

  const mock = (action: string) => fetch(`${BASE}/api/v1/test/devices/mock0002/mock`, { method: 'POST', body: JSON.stringify({ action }) });

  beforeEach(() => {
    fake = createFakeCompanion({ devices: [sampleDevice('mock0001', 'Desk'), studio], jobMs: 2 });
    vi.stubGlobal('fetch', fake.fetch);
    companion = createClient(BASE, windowSession(fake.bridge));
    jobs = createJobsStore();
  });

  afterEach(() => {
    for (const update of runners.splice(0)) update.dispose();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  test('walks every phase to the new version', async () => {
    const update = runner();
    await update.start('bundled');
    expect(update.getState().view.step).toBe('running');
    await vi.waitFor(() => expect(update.getState().view.step).toBe('done'));
    expect(update.getState().view).toMatchObject({ text: 'Updated to 1.5.0', version: '1.5.0' });
    const log = update.getState().log;
    expect(log[0]).toBe('Checking');
    expect(log.at(-1)).toBe('Firmware updated');
    const phaseOrder = ['Checking', 'Sending', 'Rebooting', 'Confirming', 'Firmware updated'];
    const seenPhases = log.map((line) => phaseOrder.findIndex((step) => line.startsWith(step)));
    expect(seenPhases).not.toContain(-1);
    expect(seenPhases).toEqual([...seenPhases].sort((a, b) => a - b));
    expect((await companion.devices.get({ device: 'mock0002' })).firmware?.version).toBe('1.5.0');
  });

  test('a rollback ends with the rollback sentence', async () => {
    await mock('rollback');
    const update = runner();
    await update.start('bundled');
    await vi.waitFor(() => expect(update.getState().view.step).toBe('failed'));
    expect(update.getState().view).toMatchObject({ code: 'firmware.rolled_back', text: failureText({ code: 'firmware.rolled_back', details: null }, STUDIO) });
    expect(update.getState().log.at(-1)).toBe('Failed: The Device rolled back to its previous Firmware.');
  });

  test('a Device that never confirms ends with the timeout sentence', async () => {
    await mock('no-confirm');
    const update = runner();
    await update.start('bundled');
    await vi.waitFor(() => expect(update.getState().view.step).toBe('failed'));
    expect(update.getState().view).toMatchObject({ code: 'firmware.confirm_timeout' });
  });

  test('a refused start fails at once', async () => {
    const offline = sampleDevice('mock0001', 'Desk');
    const update = runner(offline);
    await update.start('bundled');
    expect(update.getState().view).toEqual({ step: 'failed', text: "Desk isn't connected.", raw: 'Failed: The Device is not connected.', code: 'device.not_connected' });
  });

  test('an update keeps the build the Device runs', async () => {
    const update = runner();
    await update.start('bundled');
    await vi.waitFor(() => expect(update.getState().view.step).toBe('done'));
    expect((await companion.devices.get({ device: 'mock0002' })).firmware?.channel).toBe('normal');
  });

  test('a switch waits for consent on this PC, then installs the other build', async () => {
    const update = runner();
    await update.start('bundled', 'radio');
    await vi.waitFor(() => expect(update.getState().log).toContain('Waiting for confirmation'));
    await vi.waitFor(() => expect(update.getState().view.step).toBe('done'));
    expect(update.getState().view).toMatchObject({ text: 'Now on the radio build, 1.5.0', version: '1.5.0' });
    expect((await companion.devices.get({ device: 'mock0002' })).firmware?.channel).toBe('radio');

    const back = runner({ ...studio, firmware: studio.firmware && { ...studio.firmware, version: '1.5.0', channel: 'radio' } });
    await back.start('bundled', 'normal');
    await vi.waitFor(() => expect(back.getState().view.step).toBe('done'));
    expect(back.getState().view).toMatchObject({ text: 'Now on the normal build, 1.5.0' });
    expect((await companion.devices.get({ device: 'mock0002' })).firmware?.channel).toBe('normal');
  });

  test('a switch keeps its words when the Device reports the Job before the start returns', async () => {
    const update = runner();
    const post = companion.firmware.update.bind(companion.firmware);
    vi.spyOn(companion.firmware, 'update').mockImplementation(async (...args) => {
      const started = await post(...args);
      update.attach(started);
      return started;
    });
    await update.start('bundled', 'radio');
    await vi.waitFor(() => expect(update.getState().view.step).toBe('done'));
    expect(update.getState().view).toMatchObject({ text: 'Now on the radio build, 1.5.0' });
  });

  test('a radio file for a Device on the normal build is refused and changes nothing', async () => {
    fake.firmware.picks('radio-file');
    const update = runner();
    await update.startFromFile();
    expect(update.getState().view).toMatchObject({
      step: 'failed',
      code: 'firmware.channel_mismatch',
      text: 'That file is the radio build, and Studio runs the normal build. Nothing changed.',
    });
    expect((await companion.devices.get({ device: 'mock0002' })).firmware).toMatchObject({ version: '1.4.0', channel: 'normal' });
  });

  test('an expired bundle says to choose it again', async () => {
    const update = runner();
    await update.start('bundle_gone');
    expect(update.getState().view).toMatchObject({ step: 'failed', code: 'firmware.bundle_not_found', text: 'That file expired. Choose it again, then retry.' });
  });

  test('a busy Device follows the Job that holds it when that Job is known', async () => {
    fake.firmware.hold('mock0002');
    const first = runner();
    await first.start('bundled');
    const second = runner();
    await second.start('bundled');
    expect(second.getState().job).toBe(first.getState().job);
    fake.firmware.release('mock0002');
    await vi.waitFor(() => expect(second.getState().view.step).toBe('done'));
  });

  test('a busy Device with an unknown Job says to wait', async () => {
    const holder = job({});
    vi.spyOn(companion.firmware, 'update').mockRejectedValue(new ApiError('device.busy', 'A Job holds the Device.', { job: holder }));
    const update = runner();
    await update.start('bundled');
    expect(update.getState().view).toMatchObject({ step: 'failed', code: 'device.busy', text: 'Studio is updating. Wait for it to finish.' });
  });

  test('attach follows a Job that was already running', async () => {
    fake.firmware.hold('mock0002');
    const running = await companion.firmware.update({ device: 'mock0002' }, null, { bundle: 'bundled' });
    const update = runner();
    update.attach(running);
    expect(update.getState().job).toBe(running.id);
    await vi.waitFor(() => expect(update.getState().view).toMatchObject({ step: 'running', pct: 40 }));
    fake.firmware.release('mock0002');
    await vi.waitFor(() => expect(update.getState().view.step).toBe('done'));
  });

  test('a second start while running does nothing, and dismiss waits for the end', async () => {
    fake.firmware.hold('mock0002');
    const update = runner();
    const post = vi.spyOn(companion.firmware, 'update');
    await update.start('bundled');
    await update.start('bundled');
    expect(post).toHaveBeenCalledTimes(1);
    update.dismiss();
    expect(update.getState().view.step).toBe('running');
    fake.firmware.release('mock0002');
    await vi.waitFor(() => expect(update.getState().view.step).toBe('done'));
    update.dismiss();
    expect(update.getState()).toEqual({ view: { step: 'idle' }, job: null, log: [], fileDialogOpen: false });
  });

  test('a consent that expires while waiting shows the declined sentence, and a cancel while waiting shows it stopped', () => {
    const update = createFirmwareUpdate({ companion, jobs, context: () => ({ id: 'mock0002', ...STUDIO }), pollMs: 60_000 });
    runners.push(update);
    update.attach(job({ state: 'waiting_for_confirmation' }));
    expect(update.getState().view).toMatchObject({ step: 'waiting', text: 'Confirm on this PC to continue.' });
    jobs.getState().upsert(job({ state: 'failed', error: { code: 'firmware.declined', message: 'Firmware confirmation was declined.', details: null }, finished_ms: 1, rev: 2 }));
    expect(update.getState().view).toMatchObject({ step: 'failed', code: 'firmware.declined', text: 'Declined on this PC. Studio still runs 1.4.0.' });

    const waiting = job({ id: 'job_firmware00000002', state: 'waiting_for_confirmation' });
    update.attach(waiting);
    jobs.getState().upsert({ ...waiting, state: 'cancelled', error: { code: 'job.cancelled', message: 'The Job was cancelled.', details: null }, finished_ms: 2, rev: 2 });
    expect(update.getState().view).toEqual({ step: 'cancelled', text: 'Update stopped. Studio still runs 1.4.0.', raw: 'Cancelled' });
    expect(update.getState().log).toEqual(['Waiting for confirmation', 'Cancelled']);
  });

  test('a cancelled update stops, gives the Device back, and can start again', async () => {
    fake.firmware.hold('mock0002');
    const update = runner();
    await update.start('bundled');
    await vi.waitFor(() => expect(update.getState().view).toMatchObject({ step: 'running', pct: 40 }));
    fake.firmware.cancel('mock0002');
    await vi.waitFor(() => expect(update.getState().view.step).toBe('cancelled'));
    expect(update.getState().view).toMatchObject({ text: 'Update stopped. Studio still runs 1.4.0.' });
    const device = await companion.devices.get({ device: 'mock0002' });
    expect(device.link.state).toBe('connected');
    expect(device.firmware?.version).toBe('1.4.0');
    fake.firmware.release('mock0002');
    await update.start('bundled');
    await vi.waitFor(() => expect(update.getState().view.step).toBe('done'));
  });

  test('a file chosen on this PC updates the Device to its version', async () => {
    const update = runner();
    await update.startFromFile();
    expect(update.getState().fileDialogOpen).toBe(false);
    await vi.waitFor(() => expect(update.getState().view.step).toBe('done'));
    expect(update.getState().view).toMatchObject({ text: 'Updated to 1.5.0' });
    expect((await companion.devices.get({ device: 'mock0002' })).firmware?.version).toBe('1.5.0');
  });

  test('closing the file dialog without a file changes nothing', async () => {
    fake.firmware.picks('closed');
    const update = runner();
    await update.startFromFile();
    expect(update.getState()).toEqual({ view: { step: 'idle' }, job: null, log: [], fileDialogOpen: false });
  });

  test.each<[FakePick, string, string]>([
    ['not-firmware', 'firmware.bundle_invalid', "That file isn't a Firmware file for this Device."],
    ['dialog-open', 'system.dialog_open', 'A file dialog is already open on this PC. Close it, then retry.'],
  ])('a pick answered %s fails with its sentence and keeps the reason for Details', async (answer, code, text) => {
    fake.firmware.picks(answer);
    const update = runner();
    await update.startFromFile();
    expect(update.getState().view).toMatchObject({ step: 'failed', code, text });
    expect(update.getState().log).toHaveLength(1);
    expect(update.getState().log[0]).toMatch(/^Failed: /);
    expect((await companion.devices.get({ device: 'mock0002' })).firmware?.version).toBe('1.4.0');
  });

  test('the file dialog shows as open until it closes, and a second choose waits for it', async () => {
    const update = runner();
    const pick = vi.spyOn(companion.firmware, 'pick');
    const first = update.startFromFile();
    expect(update.getState().fileDialogOpen).toBe(true);
    await update.startFromFile();
    await first;
    expect(pick).toHaveBeenCalledTimes(1);
    expect(update.getState().fileDialogOpen).toBe(false);
    await vi.waitFor(() => expect(update.getState().view.step).toBe('done'));
  });

  test('job.updated events move the view without waiting for a poll', () => {
    const update = createFirmwareUpdate({ companion, jobs, context: () => ({ id: 'mock0002', ...STUDIO }), pollMs: 60_000 });
    runners.push(update);
    update.attach(job({ progress: progress('checking') }));
    jobs.getState().upsert(job({ progress: progress('sending', 10), rev: 2 }));
    expect(update.getState().view).toMatchObject({ text: 'Updating… 10%' });
    jobs.getState().upsert(job({ state: 'succeeded', result: { message: 'Firmware updated', version: '1.5.0' }, finished_ms: 1, rev: 3 }));
    expect(update.getState().view).toMatchObject({ step: 'done', text: 'Updated to 1.5.0' });
    expect(update.getState().log).toEqual(['Checking', 'Sending 10%', 'Firmware updated']);
  });
});
