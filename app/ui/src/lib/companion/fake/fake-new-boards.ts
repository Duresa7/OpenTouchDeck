import type { DeviceSummary, ErrorBody, FirmwareBuild, FirmwareChannel, FirmwareProgress, FirmwareResult, Job, NewBoard } from '../api.ts';
import { RADIO_CAPS } from '../samples.ts';

type Frame = Record<string, unknown> & { type: string };

export type Answer = 'accept' | 'decline';
export type BoardKind = 'silent' | 'busy' | 'other-firmware';
type Outcome = 'succeed' | 'fail';

export interface FakeNewBoardsOptions {
  devices: Map<string, DeviceSummary>;
  publish(frame: Frame): void;
  bundled: FirmwareBuild;
  boards?: NewBoard[];
  stepMs?: number;
  bootMs?: number;
}

const json = (status: number, body: unknown) =>
  new Response(status === 204 ? null : JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

const failure = (status: number, code: string, message: string, details: unknown = null) => json(status, { error: { code, message, details } });

const WRITING = [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
const IMAGE_BYTES = 1_572_864;
const CHECKING: FirmwareProgress = { phase: 'checking', pct: null, bytes: 0 };
const recovering = (pct: number): FirmwareProgress => ({ phase: 'recovering', pct, bytes: Math.round((IMAGE_BYTES * pct) / 100) });
const BOARD_KINDS: readonly BoardKind[] = ['silent', 'busy', 'other-firmware'];

export function newBoard({ id, port, kind = 'other-firmware' }: { id: string; port: string; kind?: BoardKind }): NewBoard {
  const seen_ms = Date.now();
  if (kind === 'busy') return { id, port, seen_ms, probe: 'busy', detail: 'Access is denied.' };
  if (kind === 'silent') return { id, port, seen_ms, probe: 'silent' };
  return { id, port, seen_ms, probe: 'other-firmware', product: 'esp32s3', detail: 'ESP-ROM:esp32s3-20210327' };
}

export function freshDevice({ id, port, firmware }: { id: string; port: string; firmware: { version: string; protocol: number; channel?: FirmwareChannel } }): DeviceSummary {
  return {
    id,
    rev: 1,
    name: null,
    setup: 'needs-setup',
    link: { state: 'connected', port, since_ms: Date.now(), protocol: firmware.protocol },
    carrier: 'usb',
    firmware: { channel: 'normal', ...firmware, build: 'fake', recovery: false, update_available: null },
    // The radio build says what it can do from its first hello, so its setup can offer Wi-Fi.
    capabilities: firmware.channel === 'radio' ? RADIO_CAPS : null,
    diagnostics: null,
    orientation: 'landscape',
    layout: null,
    control_layout: { state: 'unavailable', reason: 'checking' },
    key_mirror: { enabled: true, reason: null },
    stable_id: true,
  };
}

function plugOf(body: unknown): { id: string; kind: string } | null {
  if (typeof body !== 'object' || body === null || !('plug' in body)) return null;
  const plug = body.plug;
  if (typeof plug !== 'object' || plug === null || !('id' in plug) || !('kind' in plug)) return null;
  return typeof plug.id === 'string' && typeof plug.kind === 'string' ? { id: plug.id, kind: plug.kind } : null;
}

export function createFakeNewBoards(options: FakeNewBoardsOptions) {
  const { devices, publish, bundled, stepMs = 120, bootMs = 400 } = options;
  const boards = new Map((options.boards ?? []).map((b) => [b.id, b]));
  const jobs = new Map<string, Job>();
  const confirming = new Map<string, (answer: Answer) => void>();
  const script: Answer[] = [];
  const outcomes = new Map<string, Outcome>();
  const held = new Map<string, number | 'done'>();
  const waiting = new Map<string, () => void>();
  let minted = 0;

  function update(job: Job, change: Partial<Job>) {
    const next = { ...job, ...change, rev: job.rev + 1 };
    jobs.set(next.id, next);
    publish({ type: 'job.updated', data: next });
    return next;
  }

  const active = (id: string) =>
    [...jobs.values()].find((job) => job.scope.kind === 'new-board' && job.scope.id === id && job.finished_ms === null);

  function end(id: string, outcome: { result: FirmwareResult } | { error: ErrorBody }) {
    const job = jobs.get(id);
    if (!job || job.finished_ms !== null) return;
    const done = Date.now();
    update(job, 'result' in outcome ? { state: 'succeeded', result: outcome.result, cancellable: false, finished_ms: done } : { state: 'failed', error: outcome.error, cancellable: false, finished_ms: done });
  }

  function becomeDevice(board: NewBoard) {
    boards.delete(board.id);
    publish({ type: 'new-board.removed', data: { id: board.id } });
    const add = () => {
      const device = freshDevice({ id: board.id, port: board.port, firmware: { version: bundled.version, protocol: 2 } });
      devices.set(device.id, device);
      publish({ type: 'device.added', device: device.id, data: device });
    };
    if (held.get(board.id) === 'done') {
      waiting.set(board.id, () => setTimeout(add, bootMs));
      return;
    }
    setTimeout(add, bootMs);
  }

  function write(jobId: string, board: NewBoard) {
    const steps: FirmwareProgress[] = [CHECKING, ...WRITING.map(recovering)];
    let at = 0;
    const tick = () => {
      const job = jobs.get(jobId);
      if (!job || job.finished_ms !== null) return;
      if (at < steps.length) {
        const progress = steps[at++];
        update(job, { state: 'running', progress, cancellable: false });
        if (progress.pct !== null && progress.pct === held.get(board.id)) {
          waiting.set(board.id, () => setTimeout(tick, stepMs));
          return;
        }
        setTimeout(tick, stepMs);
        return;
      }
      if (outcomes.get(board.id) === 'fail') {
        end(jobId, { error: { code: 'firmware.failed', message: 'The update failed.', details: { reason: 'recovery port not found' } } });
        return;
      }
      end(jobId, { result: { message: 'Firmware installed', version: bundled.version } });
      becomeDevice(board);
    };
    setTimeout(tick, stepMs);
  }

  function ask(jobId: string, board: NewBoard) {
    const answer = (given: Answer) => {
      confirming.delete(board.id);
      const job = jobs.get(jobId);
      if (!job || job.finished_ms !== null) return;
      if (given === 'decline') end(jobId, { error: { code: 'firmware.declined', message: 'Firmware confirmation was declined.', details: null } });
      else write(jobId, board);
    };
    const scripted = script.shift();
    if (scripted) setTimeout(() => answer(scripted), stepMs);
    else confirming.set(board.id, answer);
  }

  async function install(request: Request, id: string): Promise<Response> {
    const board = boards.get(id);
    if (!board) return failure(404, 'new_board.not_found', 'No New board with that id.');
    const holding = active(id);
    if (holding) return failure(409, 'device.busy', 'A Job holds the Device.', { job: holding });
    const body: unknown = await request.json().catch(() => null);
    const bundle = typeof body === 'object' && body !== null && 'bundle' in body ? body.bundle : null;
    if (bundle !== 'bundled') return failure(404, 'firmware.bundle_not_found', 'The bundle expired, was used or belongs to another Device.');
    minted += 1;
    const job: Job = {
      id: `job_install${String(minted).padStart(9, '0')}`,
      kind: 'firmware.install',
      scope: { kind: 'new-board', id },
      state: 'waiting_for_confirmation',
      progress: CHECKING,
      result: null,
      error: null,
      cancellable: true,
      created_ms: Date.now(),
      finished_ms: null,
      rev: 1,
    };
    jobs.set(job.id, job);
    publish({ type: 'job.updated', data: job });
    ask(job.id, board);
    return json(202, job);
  }

  function plug(board: NewBoard) {
    boards.set(board.id, board);
    publish({ type: 'new-board.added', data: board });
  }

  function unplug(id: string) {
    if (!boards.delete(id)) return;
    publish({ type: 'new-board.removed', data: { id } });
  }

  return {
    async handle(request: Request, path: string): Promise<Response | null> {
      const method = request.method;
      let match: RegExpExecArray | null;
      if (path === '/new-boards') return method === 'GET' ? json(200, [...boards.values()]) : failure(405, 'route.method_not_allowed', 'Use GET.');
      if ((match = /^\/new-boards\/([^/]+)\/install$/.exec(path))) {
        return method === 'POST' ? install(request, decodeURIComponent(match[1])) : failure(405, 'route.method_not_allowed', 'Use POST.');
      }
      if ((match = /^\/jobs\/([^/]+)$/.exec(path))) {
        const job = jobs.get(decodeURIComponent(match[1]));
        if (!job) return null;
        if (method === 'GET') return json(200, job);
        if (method !== 'DELETE') return failure(405, 'route.method_not_allowed', 'Use GET or DELETE.');
        if (job.state !== 'waiting_for_confirmation') return failure(409, 'job.not_cancellable', "The Job's current phase can't be cancelled.");
        if (job.scope.kind === 'new-board') confirming.delete(job.scope.id);
        const error = { code: 'job.cancelled', message: 'The Job was cancelled.', details: null };
        return json(200, update(job, { state: 'cancelled', error, cancellable: false, finished_ms: Date.now() }));
      }
      return null;
    },

    async test(request: Request, path: string): Promise<Response | null> {
      if (path !== '/test/ports' || request.method !== 'POST') return null;
      const body: unknown = await request.json().catch(() => null);
      const plugged = plugOf(body);
      const kind = BOARD_KINDS.find((k) => k === plugged?.kind);
      if (plugged && kind) {
        plug(newBoard({ id: plugged.id, port: `COM${10 + boards.size}`, kind }));
        return json(204, null);
      }
      if (typeof body === 'object' && body !== null && 'unplug' in body && typeof body.unplug === 'string') {
        unplug(body.unplug);
        return json(204, null);
      }
      return failure(400, 'request.invalid', 'Unknown port change.', { part: 'body', reason: 'plug a silent, busy or other-firmware board, or unplug one' });
    },

    boards: () => [...boards.values()],
    jobs: () => [...jobs.values()],
    plug,
    unplug,
    confirming: (id: string) => confirming.has(id),
    answer(id: string, given: Answer) {
      confirming.get(id)?.(given);
    },
    script: (...answers: Answer[]) => void script.push(...answers),
    outcome: (id: string, outcome: Outcome) => void outcomes.set(id, outcome),
    hold: (id: string, at: number | 'done' = 40) => void held.set(id, at),
    release(id: string) {
      held.delete(id);
      waiting.get(id)?.();
      waiting.delete(id);
    },
  };
}

export type FakeNewBoards = ReturnType<typeof createFakeNewBoards>;
