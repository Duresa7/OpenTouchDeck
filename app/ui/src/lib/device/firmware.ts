import { createStore } from 'zustand/vanilla';
import type { ErrorBody, FirmwareChannel, FirmwareProgress, FirmwareRequest, FirmwareResult, Job, PickResult } from '../companion/api.ts';
import type { Companion } from '../companion/client.ts';
import { toApiError } from '../companion/errors.ts';
import type { JobsStore } from '../companion/jobs.ts';

export type JobView =
  | { step: 'waiting'; text: string; raw: string }
  | { step: 'running'; text: string; raw: string; pct: number | null }
  | { step: 'done'; text: string; raw: string; version: string }
  | { step: 'failed'; text: string; raw: string; code: string }
  | { step: 'cancelled'; text: string; raw: string };

export type UpdateView = { step: 'idle' } | JobView;

export interface UpdateContext {
  device: string;
  version: string;
  /** The build a switch installs. Absent for an update within the build the Device runs. */
  switchTo?: FirmwareChannel;
}

type BundleRef = FirmwareRequest['bundle'];
const PHASES = ['checking', 'sending', 'rebooting', 'confirming', 'recovering'] as const;
type Phase = (typeof PHASES)[number];
export type PhaseProgress = Pick<FirmwareProgress, 'pct'> & { phase: Phase };

const RAW: Record<Phase, string> = { checking: 'Checking', sending: 'Sending', rebooting: 'Rebooting', confirming: 'Confirming', recovering: 'Recovering' };

function progressOf(value: unknown): PhaseProgress | null {
  if (typeof value !== 'object' || value === null || !('phase' in value)) return null;
  const phase = PHASES.find((p) => p === value.phase);
  if (!phase) return null;
  return { phase, pct: 'pct' in value && typeof value.pct === 'number' ? value.pct : null };
}

function resultOf(value: unknown): FirmwareResult | null {
  if (typeof value !== 'object' || value === null || !('message' in value) || !('version' in value)) return null;
  return typeof value.message === 'string' && typeof value.version === 'string' ? { message: value.message, version: value.version } : null;
}

const reasonOf = (details: unknown): string =>
  typeof details === 'object' && details !== null && 'reason' in details && typeof details.reason === 'string' ? details.reason : '';

const channelIn = (details: unknown, key: 'channel' | 'bundle'): FirmwareChannel =>
  typeof details === 'object' && details !== null && Reflect.get(details, key) === 'radio' ? 'radio' : 'normal';

export function phaseText(progress: PhaseProgress | null): string {
  if (!progress) return 'Updating…';
  const pct = Math.round(progress.pct ?? 0);
  switch (progress.phase) {
    case 'checking':
      return 'Checking the file…';
    case 'sending':
      return `Updating… ${pct}%`;
    case 'recovering':
      return `Repairing… ${pct}%`;
    case 'rebooting':
      return 'Restarting…';
    case 'confirming':
      return 'Checking that it started…';
    default: {
      const _exhaustive: never = progress.phase;
      return _exhaustive;
    }
  }
}

export function failureText(error: { code: string; details: unknown }, { device, version }: UpdateContext): string {
  const reason = reasonOf(error.details);
  switch (error.code) {
    case 'firmware.rolled_back':
      return `The new Firmware didn't start, so ${device} went back to ${version}. It still works. Retry, or repair it from Troubleshooting.`;
    case 'firmware.confirm_timeout':
      return `${device} didn't come back after restarting. Unplug it and plug it back in. If its screen stays blank, repair it from Troubleshooting.`;
    case 'firmware.declined':
      return `Declined on this PC. ${device} still runs ${version}.`;
    case 'firmware.channel_mismatch':
      return `That file is the ${channelIn(error.details, 'bundle')} build, and ${device} runs the ${channelIn(error.details, 'channel')} build. Nothing changed.`;
    case 'firmware.bundle_invalid':
      return "That file isn't a Firmware file for this Device.";
    case 'firmware.bundle_not_found':
      return 'That file expired. Choose it again, then retry.';
    case 'system.dialog_open':
      return 'A file dialog is already open on this PC. Close it, then retry.';
    case 'device.busy':
      return `${device} is updating. Wait for it to finish.`;
    case 'device.not_connected':
      return `${device} isn't connected.`;
    case 'job.cancelled':
      return `Update stopped. ${device} still runs ${version}.`;
    case 'firmware.failed':
      if (reason.includes('unplugged')) return `${device} was unplugged during the update. Plug it back in. If its screen stays blank, repair it from Troubleshooting.`;
      if (reason.includes('too many chunks')) return "That file is over 6 MB, so it isn't a Firmware file.";
      return `Couldn't update. ${device} still runs ${version}. Check the cable, then retry.`;
    default:
      return `Couldn't update. ${device} still runs ${version}. Check the cable, then retry.`;
  }
}

export function doneText(result: FirmwareResult, context?: UpdateContext): string {
  if (context?.switchTo) return `Now on the ${context.switchTo} build, ${result.version}`;
  if (result.message === 'Firmware updated') return `Updated to ${result.version}`;
  if (result.message === 'Already on this build') return `Already on Firmware ${result.version}`;
  return `Done. ${result.message}.`;
}

export interface FirmwareWords {
  waiting: string;
  progress(progress: PhaseProgress | null): string;
  done(result: FirmwareResult, context?: UpdateContext): string;
  failed(error: { code: string; details: unknown }, context: UpdateContext): string;
}

export const updateWords: FirmwareWords = { waiting: 'Confirm on this PC to continue.', progress: phaseText, done: doneText, failed: failureText };

export const inProgress = (view: UpdateView) => view.step === 'running' || view.step === 'waiting';

export function viewOf(job: Job, context: UpdateContext, words: FirmwareWords = updateWords): JobView {
  switch (job.state) {
    case 'queued':
      return { step: 'running', text: words.progress(null), raw: 'Queued', pct: null };
    case 'waiting_for_confirmation':
      return { step: 'waiting', text: words.waiting, raw: 'Waiting for confirmation' };
    case 'running': {
      const progress = progressOf(job.progress);
      const pct = progress?.pct ?? null;
      return { step: 'running', text: words.progress(progress), raw: progress ? `${RAW[progress.phase]}${pct === null ? '' : ` ${pct}%`}` : 'Running', pct };
    }
    case 'succeeded': {
      const result = resultOf(job.result) ?? { message: 'Firmware updated', version: context.version };
      return { step: 'done', text: words.done(result, context), raw: result.message, version: result.version };
    }
    case 'failed': {
      const error: ErrorBody = job.error ?? { code: 'internal', message: 'The Job ended as failed.', details: null };
      return { step: 'failed', text: words.failed(error, context), raw: `Failed: ${error.message}`, code: error.code };
    }
    case 'cancelled':
      return { step: 'cancelled', text: words.failed({ code: 'job.cancelled', details: null }, context), raw: 'Cancelled' };
    default: {
      const _exhaustive: never = job.state;
      return _exhaustive;
    }
  }
}

export interface FirmwareUpdateState {
  view: UpdateView;
  job: string | null;
  log: string[];
  fileDialogOpen: boolean;
}

export interface FirmwareJobDeps {
  companion: Pick<Companion, 'jobs'>;
  jobs: JobsStore;
  context(): UpdateContext & { id: string };
  run(id: string, bundle: BundleRef, channel?: FirmwareChannel): Promise<Job>;
  words?: FirmwareWords;
  pollMs?: number;
}

export interface FirmwareUpdateDeps extends Omit<FirmwareJobDeps, 'companion' | 'run' | 'words'> {
  companion: Pick<Companion, 'firmware' | 'jobs'>;
}

const finished = (job: Job) => job.state === 'succeeded' || job.state === 'failed' || job.state === 'cancelled';

export function createFirmwareJob(deps: FirmwareJobDeps) {
  const { companion, jobs, words = updateWords, pollMs = 1000 } = deps;
  const store = createStore<FirmwareUpdateState>()(() => ({ view: { step: 'idle' }, job: null, log: [], fileDialogOpen: false }));
  const { getState: state, setState: set } = store;
  let stop = () => {};
  let started: UpdateContext = { device: 'the Device', version: '' };
  // While a start waits for its Job, the Device may already report it. The start follows it, with what it asked for.
  let starting = false;

  function show(job: Job) {
    if (state().job !== job.id) return;
    const view = viewOf(job, started, words);
    set((s) => ({ view, log: s.log.at(-1) === view.raw ? s.log : [...s.log, view.raw] }));
    if (finished(job)) stop();
  }

  function follow(job: Job) {
    stop();
    set({ job: job.id, log: [] });
    const unsubscribe = jobs.subscribe((s) => {
      const latest = s.jobs[job.id];
      if (latest) show(latest);
    });
    const timer = setInterval(() => {
      companion.jobs.get({ job: job.id }).then(
        (latest) => jobs.getState().upsert(latest),
        () => {},
      );
    }, pollMs);
    stop = () => {
      unsubscribe();
      clearInterval(timer);
    };
    jobs.getState().upsert(job);
    show(jobs.getState().jobs[job.id] ?? job);
  }

  return Object.assign(store, {
    /** Starts an update. Naming a `channel` other than the one the Device runs asks to switch builds. */
    async start(bundle: BundleRef = 'bundled', channel?: FirmwareChannel): Promise<void> {
      if (inProgress(state().view)) return;
      const { id, ...context } = deps.context();
      started = channel ? { ...context, switchTo: channel } : context;
      set({ view: { step: 'running', text: words.progress(null), raw: 'Starting', pct: null }, job: null, log: [] });
      starting = true;
      try {
        const job = await deps.run(id, bundle, channel);
        starting = false;
        follow(job);
      } catch (thrown) {
        starting = false;
        const error = toApiError(thrown);
        const busy = error.code === 'device.busy' && typeof error.details === 'object' && error.details !== null && 'job' in error.details ? error.details.job : null;
        if (busy && typeof busy === 'object' && 'id' in busy && typeof busy.id === 'string') {
          const known = jobs.getState().jobs[busy.id];
          if (known) return follow(known);
        }
        set({ view: { step: 'failed', text: words.failed(error, context), raw: `Failed: ${error.message}`, code: error.code } });
      }
    },

    attach(job: Job) {
      if (starting || state().job === job.id) return;
      const { id: _id, ...context } = deps.context();
      started = context;
      follow(job);
    },

    dismiss() {
      if (!inProgress(state().view)) set({ view: { step: 'idle' }, job: null, log: [] });
    },

    dispose() {
      stop();
    },
  });
}

export function createFirmwareUpdate(deps: FirmwareUpdateDeps) {
  const { companion } = deps;
  const update = createFirmwareJob({
    ...deps,
    run: (id, bundle, channel) => companion.firmware.update({ device: id }, null, channel ? { bundle, channel } : { bundle }),
  });
  const { getState: state, setState: set } = update;
  return Object.assign(update, {
    async startFromFile(): Promise<void> {
      if (state().fileDialogOpen || inProgress(state().view)) return;
      const { id, ...context } = deps.context();
      set({ fileDialogOpen: true });
      let picked: PickResult;
      try {
        picked = await companion.firmware.pick({}, null, { device: id });
      } catch (thrown) {
        const error = toApiError(thrown);
        const raw = `Failed: ${error.message}`;
        set({ view: { step: 'failed', text: failureText(error, context), raw, code: error.code }, job: null, log: [raw] });
        return;
      } finally {
        set({ fileDialogOpen: false });
      }
      if (picked.bundle) await update.start(picked.bundle.id);
    },
  });
}

export type FirmwareUpdateStore = ReturnType<typeof createFirmwareUpdate>;
