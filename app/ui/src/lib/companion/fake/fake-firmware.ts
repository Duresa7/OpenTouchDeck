import type { BundleInfo, DeviceSummary, ErrorBody, FirmwareBuild, FirmwareChannel, FirmwareProgress, FirmwareResult, Job, LinkState } from '../api.ts';

type Frame = Record<string, unknown> & { type: string };

export interface FakeFirmwareOptions {
  devices: Map<string, DeviceSummary>;
  publish(frame: Frame): void;
  bundled?: FirmwareBuild;
  stepMs?: number;
}

type Mode = 'normal-update' | 'rollback' | 'no-confirm';
export type FakePick = 'firmware-file' | 'radio-file' | 'closed' | 'not-firmware' | 'dialog-open';
type Target = { version: string; channel: FirmwareChannel };

/** What the fake Companion carries unless a test says otherwise: the normal build and the radio build. */
export const FAKE_BUNDLED: FirmwareBuild = { version: '1.5.0', build: 'fake', radio: { version: '1.5.0', build: 'fake-radio' } };

const isChannel = (value: unknown): value is FirmwareChannel => value === 'normal' || value === 'radio';
const MODES: readonly Mode[] = ['normal-update', 'rollback', 'no-confirm'];

const json = (status: number, body: unknown) =>
  new Response(status === 204 ? null : JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

const failure = (status: number, code: string, message: string, details: unknown = null) => json(status, { error: { code, message, details } });

const SENDING = [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
const BUNDLE_BYTES = 1_572_864;

const phase = (name: 'checking' | 'rebooting' | 'confirming'): FirmwareProgress => ({ phase: name, pct: null, bytes: 0 });
const sending = (pct: number): FirmwareProgress => ({ phase: 'sending', pct, bytes: Math.round((BUNDLE_BYTES * pct) / 100) });

export function createFakeFirmware(options: FakeFirmwareOptions) {
  const { devices, publish, stepMs = 120 } = options;
  const bundled = options.bundled ?? FAKE_BUNDLED;
  const jobs = new Map<string, Job>();
  const bundles = new Map<string, BundleInfo & { device: string }>();
  const modes = new Map<string, Mode>();
  const held = new Map<string, number | 'rebooting'>();
  const waiting = new Map<string, () => void>();
  const ports = new Map<string, string>();
  const picks: FakePick[] = [];
  let minted = 0;

  function setDevice(id: string, change: Partial<DeviceSummary>) {
    const current = devices.get(id);
    if (!current) return;
    const next = { ...current, ...change, rev: current.rev + 1 };
    devices.set(id, next);
    publish({ type: 'device.updated', device: id, data: next });
  }

  function update(job: Job, change: Partial<Job>) {
    const next = { ...job, ...change, rev: job.rev + 1 };
    jobs.set(next.id, next);
    publish({ type: 'job.updated', data: next });
    return next;
  }

  const active = (device: string) =>
    [...jobs.values()].find((job) => job.scope.kind === 'device' && job.scope.id === device && job.finished_ms === null);

  const connected = (port: string): LinkState => ({ state: 'connected', port, since_ms: Date.now(), protocol: 2 });

  function finish(id: string, port: string, outcome: { result: FirmwareResult } | { error: ErrorBody }, target: Target | null) {
    const job = jobs.get(id);
    if (!job || job.finished_ms !== null) return;
    const device = devices.get(job.scope.kind === 'device' ? job.scope.id : '');
    if (device?.firmware) {
      const firmware = target
        ? { ...device.firmware, version: target.version, channel: target.channel, protocol: Math.max(device.firmware.protocol, 2), update_available: null }
        : device.firmware;
      setDevice(device.id, { firmware, link: connected(port) });
    }
    update(job, 'result' in outcome
      ? { state: 'succeeded', result: outcome.result, cancellable: false, finished_ms: Date.now() }
      : { state: 'failed', error: outcome.error, cancellable: false, finished_ms: Date.now() });
  }

  function run(id: string, device: DeviceSummary, port: string, target: Target) {
    const mode = modes.get(device.id) ?? 'normal-update';
    const same = device.firmware?.version === target.version && device.firmware.channel === target.channel;
    const steps: FirmwareProgress[] = same ? [phase('checking')] : [phase('checking'), ...SENDING.map(sending), phase('rebooting'), phase('confirming')];
    let at = 0;
    const tick = () => {
      const job = jobs.get(id);
      if (!job || job.finished_ms !== null) return;
      if (at < steps.length) {
        const progress = steps[at++];
        update(job, { progress, cancellable: progress.phase === 'checking' || progress.phase === 'sending' });
        if (progress.phase === held.get(device.id) || (progress.phase === 'sending' && progress.pct === held.get(device.id))) {
          waiting.set(device.id, () => setTimeout(tick, stepMs));
          return;
        }
        setTimeout(tick, stepMs);
        return;
      }
      if (same) finish(id, port, { result: { message: 'Already on this build', version: target.version } }, null);
      else if (mode === 'rollback') finish(id, port, { error: { code: 'firmware.rolled_back', message: 'The Device rolled back to its previous Firmware.', details: null } }, null);
      else if (mode === 'no-confirm') finish(id, port, { error: { code: 'firmware.confirm_timeout', message: "The new Firmware didn't confirm in time.", details: null } }, null);
      else finish(id, port, { result: { message: 'Firmware updated', version: target.version } }, target);
    };
    setTimeout(tick, stepMs);
  }

  function cancel(job: Job): Job {
    const device = devices.get(job.scope.kind === 'device' ? job.scope.id : '');
    if (device && device.link.state === 'updating') setDevice(device.id, { link: connected(ports.get(job.id) ?? '') });
    const error = { code: 'job.cancelled', message: 'The Job was cancelled.', details: null };
    return update(job, { state: 'cancelled', error, cancellable: false, finished_ms: Date.now() });
  }

  async function startUpdate(request: Request, id: string): Promise<Response> {
    const device = devices.get(id);
    if (!device) return failure(404, 'device.not_found', 'No Device with that id.');
    const holding = active(id);
    if (holding) return failure(409, 'device.busy', 'A Job holds the Device.', { job: holding });
    if (device.link.state !== 'connected') return failure(409, 'device.not_connected', 'The Device is not connected.', { link: device.link });
    const body: unknown = await request.json().catch(() => null);
    const bundle = typeof body === 'object' && body !== null && 'bundle' in body && typeof body.bundle === 'string' ? body.bundle : null;
    if (bundle === null) return failure(400, 'request.invalid', 'The body needs a bundle.', { part: 'body', reason: 'bundle' });
    const asked = typeof body === 'object' && body !== null && 'channel' in body ? body.channel : undefined;
    if (asked !== undefined && asked !== null && !isChannel(asked)) return failure(400, 'request.invalid', 'Unknown channel.', { part: 'body', reason: 'channel' });
    // As the Companion does: the Device's own build picks the bundle, and only a request naming the other build switches.
    const running = device.firmware?.channel ?? 'normal';
    const channel = isChannel(asked) ? asked : running;
    let target: Target;
    if (bundle === 'bundled') {
      const version = channel === 'radio' ? bundled.radio?.version : bundled.version;
      if (!version) return failure(422, 'firmware.bundle_invalid', 'The Firmware bundle is invalid or unavailable.');
      target = { version, channel };
    } else {
      const picked = bundles.get(bundle);
      if (!picked || picked.device !== id) return failure(404, 'firmware.bundle_not_found', 'The bundle expired, was used or belongs to another Device.');
      if (picked.channel !== channel) {
        return failure(409, 'firmware.channel_mismatch', `The bundle is the ${picked.channel} build, not the ${channel} build.`, { channel, bundle: picked.channel });
      }
      bundles.delete(bundle);
      target = picked;
    }
    const switching = channel !== running;
    minted += 1;
    const job: Job = {
      id: `job_firmware${String(minted).padStart(8, '0')}`,
      kind: 'firmware.update',
      scope: { kind: 'device', id },
      // A switch waits for the PC's consent dialog first, which the fake answers yes after one step.
      state: switching ? 'waiting_for_confirmation' : 'running',
      progress: switching ? null : phase('checking'),
      result: null,
      error: null,
      cancellable: true,
      created_ms: Date.now(),
      finished_ms: null,
      rev: 1,
    };
    jobs.set(job.id, job);
    ports.set(job.id, device.link.port);
    publish({ type: 'job.updated', data: job });
    setDevice(id, { link: { state: 'updating', job: job.id } });
    if (switching) setTimeout(() => run(job.id, device, device.link.state === 'connected' ? device.link.port : '', target), stepMs);
    else run(job.id, device, device.link.port, target);
    return json(202, job);
  }

  return {
    async handle(request: Request, path: string): Promise<Response | null> {
      const method = request.method;
      let match: RegExpExecArray | null;
      if (path === '/firmware/bundled') return method === 'GET' ? json(200, bundled) : failure(405, 'route.method_not_allowed', 'Use GET.');
      if (path === '/firmware/bundles/pick') {
        if (method !== 'POST') return failure(405, 'route.method_not_allowed', 'Use POST.');
        const body: unknown = await request.json().catch(() => null);
        const device = typeof body === 'object' && body !== null && 'device' in body && typeof body.device === 'string' ? body.device : '';
        if (!devices.has(device)) return failure(404, 'device.not_found', 'No Device with that id.');
        const answer = picks.shift() ?? 'firmware-file';
        if (answer === 'closed') return json(200, { bundle: null });
        if (answer === 'not-firmware') return failure(422, 'firmware.bundle_invalid', 'The Firmware bundle is invalid or unavailable.');
        if (answer === 'dialog-open') return failure(409, 'system.dialog_open', 'A native file dialog is already open.');
        minted += 1;
        const radio = answer === 'radio-file' && bundled.radio;
        const info: BundleInfo = radio
          ? { id: `bundle_fake${String(minted).padStart(8, '0')}`, version: radio.version, build: radio.build, bytes: BUNDLE_BYTES, channel: 'radio' }
          : { id: `bundle_fake${String(minted).padStart(8, '0')}`, version: bundled.version, build: bundled.build, bytes: BUNDLE_BYTES, channel: 'normal' };
        bundles.set(info.id, { ...info, device });
        return json(200, { bundle: info });
      }
      if ((match = /^\/devices\/([^/]+)\/firmware\/update$/.exec(path))) {
        return method === 'POST' ? startUpdate(request, decodeURIComponent(match[1])) : failure(405, 'route.method_not_allowed', 'Use POST.');
      }
      if ((match = /^\/jobs\/([^/]+)$/.exec(path))) {
        const job = jobs.get(decodeURIComponent(match[1]));
        if (!job) return null;
        if (method === 'GET') return json(200, job);
        if (method !== 'DELETE') return failure(405, 'route.method_not_allowed', 'Use GET or DELETE.');
        if (!job.cancellable || job.finished_ms !== null) return failure(409, 'job.not_cancellable', "The Job's current phase can't be cancelled.");
        return json(200, cancel(job));
      }
      return null;
    },

    async test(request: Request, path: string): Promise<Response | null> {
      const match = /^\/test\/devices\/([^/]+)\/mock$/.exec(path);
      if (!match || request.method !== 'POST') return null;
      const body: unknown = await request.json().catch(() => null);
      const action = typeof body === 'object' && body !== null && 'action' in body ? body.action : null;
      const mode = MODES.find((m) => m === action);
      if (!mode) return failure(400, 'request.invalid', 'Unknown mock action.', { part: 'body', reason: 'unknown mock action' });
      modes.set(decodeURIComponent(match[1]), mode);
      return json(204, null);
    },

    jobs: () => [...jobs.values()],
    picks: (...answers: FakePick[]) => void picks.splice(0, picks.length, ...answers),
    hold: (device: string, at: number | 'rebooting' = 40) => void held.set(device, at),
    cancel(device: string) {
      const job = active(device);
      if (job?.cancellable) cancel(job);
    },
    release(device: string) {
      held.delete(device);
      waiting.get(device)?.();
      waiting.delete(device);
    },
  };
}
