import { createStore, type StoreApi } from 'zustand/vanilla';
import type { DeviceSettingsPatch, DeviceSummary, Job, PushReport } from '../companion/api.ts';
import type { Companion } from '../companion/client.ts';
import { toApiError } from '../companion/errors.ts';
import { readDraft, writeDraft, type DraftStorage } from '../editor/store.ts';
import { canonical, counts, newLayout, parseLayout, starterLayout, type Layout, type Orientation } from '../layout/model.ts';
import { offersWifi } from './radio-build.ts';
import { NAME_BYTES, nameBreak, SPECIAL_CHARACTERS, type DeviceSettingsStore } from './settings.ts';

export type StepId = 'name' | 'orientation' | 'layout' | 'firmware' | 'wifi' | 'done';
export type LayoutChoice = 'starter' | 'blank' | 'keep';

export interface Flow {
  steps: readonly StepId[];
  at: StepId;
}

export type FlowEvent = { type: 'next' } | { type: 'back' } | { type: 'offer'; firmwareFirst: boolean };

const SETUP: readonly StepId[] = ['name', 'orientation', 'layout'];

/** The steps in order. The Wi-Fi step, only for a Device on the radio build, comes last before done. */
function stepsOf(firmware: boolean, firmwareFirst: boolean, wifi: boolean): readonly StepId[] {
  const steps: StepId[] = !firmware ? [...SETUP] : firmwareFirst ? ['firmware', ...SETUP] : [...SETUP, 'firmware'];
  return wifi ? [...steps, 'wifi', 'done'] : [...steps, 'done'];
}

export function startFlow({ firmware, firmwareFirst, wifi = false }: { firmware: boolean; firmwareFirst: boolean; wifi?: boolean }): Flow {
  const steps = stepsOf(firmware, firmwareFirst, wifi);
  return { steps, at: steps[0] };
}

export function flowStep(flow: Flow, event: FlowEvent): Flow {
  const index = flow.steps.indexOf(flow.at);
  switch (event.type) {
    case 'next':
      return { ...flow, at: flow.steps[Math.min(index + 1, flow.steps.length - 1)] };
    case 'back':
      return { ...flow, at: flow.steps[Math.max(index - 1, 0)] };
    case 'offer': {
      if (flow.steps.includes('firmware')) return flow;
      const first = event.firmwareFirst && index === 0;
      return { steps: stepsOf(true, first, flow.steps.includes('wifi')), at: first ? 'firmware' : flow.at };
    }
    default: {
      const _exhaustive: never = event;
      return _exhaustive;
    }
  }
}

export function stepCount(flow: Flow): { n: number; total: number } {
  return { n: flow.steps.indexOf(flow.at) + 1, total: flow.steps.length };
}

export function nameError(name: string): string | null {
  const broken = nameBreak(name);
  switch (broken) {
    case null:
      return null;
    case 'empty':
      return 'Enter a name.';
    case 'too-long':
      return `Names can be up to ${NAME_BYTES} characters.`;
    case 'special':
      return SPECIAL_CHARACTERS;
    default: {
      const _exhaustive: never = broken;
      return _exhaustive;
    }
  }
}

export interface OnDevice {
  layout: Layout;
  revision: string;
}

export type Busy = 'saving' | 'restarting' | 'reading' | 'sending';

export interface SetupState {
  flow: Flow;
  name: string;
  nameError: string | null;
  needsFirmware: boolean;
  orientation: Orientation;
  onDevice: OnDevice | null;
  choice: LayoutChoice;
  busy: Busy | null;
  error: { text: string; detail: string } | null;
}

export interface SetupDeps {
  companion: Pick<Companion, 'devices' | 'layouts'>;
  wait(job: Job): Promise<PushReport>;
  settings: Pick<DeviceSettingsStore, 'getState' | 'subscribe' | 'chooseOrientation' | 'confirmOrientation'>;
  drafts: DraftStorage | null;
  device: DeviceSummary;
  firmware: { offered: string | null };
}

const NO_CHANGE: DeviceSettingsPatch = { idle_timeout_s: null, orientation: null, name: null, brightness: null, photo_frame: null };
const PLUG_IN = "Check that it's plugged in, then retry.";

const orientationOf = (device: DeviceSummary): Orientation => (device.orientation === 'portrait' ? 'portrait' : 'landscape');

function until<T>(store: Pick<StoreApi<T>, 'getState' | 'subscribe'>, done: (state: T) => boolean): Promise<T> {
  return new Promise((resolve) => {
    if (done(store.getState())) return resolve(store.getState());
    const stop = store.subscribe((state) => {
      if (!done(state)) return;
      stop();
      resolve(state);
    });
  });
}

export function createSetup(deps: SetupDeps) {
  const { companion, settings } = deps;
  let device = deps.device;
  const v1 = (device.firmware?.protocol ?? 1) < 2;
  const store = createStore<SetupState>()(() => ({
    flow: startFlow({ firmware: deps.firmware.offered !== null, firmwareFirst: v1, wifi: offersWifi(device) }),
    name: device.name ?? '',
    nameError: null,
    needsFirmware: false,
    orientation: orientationOf(device),
    onDevice: null,
    choice: 'starter',
    busy: null,
    error: null,
  }));
  const { getState: state, setState: set } = store;
  const called = () => device.name ?? 'the Device';
  const move = (type: 'next' | 'back') => set((s) => ({ flow: flowStep(s.flow, { type }), error: null }));

  async function readOnDevice(): Promise<void> {
    const orientation = state().orientation;
    set({ busy: 'reading', onDevice: null });
    try {
      const read = await companion.layouts.get({ device: device.id, orientation });
      const parsed = parseLayout(JSON.stringify(read.layout));
      if (state().orientation !== orientation) return;
      set(parsed.ok ? { onDevice: { layout: parsed.layout, revision: read.revision }, choice: 'keep' } : { choice: 'starter' });
    } catch {
      set({ choice: 'starter' });
    } finally {
      set({ busy: null });
    }
  }

  function keepDraft(layout: Layout, revision: string | null, replace: boolean) {
    const { drafts } = deps;
    if (!drafts) return;
    const orientation = layout.orientation;
    if (!replace && readDraft(drafts, device.id, orientation)) return;
    writeDraft(drafts, device.id, orientation, { layout, onDevice: canonical(layout), revision });
  }

  const api = {
    setName(name: string) {
      set({ name, nameError: null, error: null });
    },

    async submitName(): Promise<void> {
      if (state().busy) return;
      if (state().needsFirmware) return move('next');
      const name = state().name.trim();
      const problem = nameError(name);
      if (problem) return set({ nameError: problem });
      if (name === device.name) return move('next');
      set({ busy: 'saving', error: null });
      try {
        await companion.devices.settings_patch({ device: device.id }, null, { ...NO_CHANGE, name });
        set({ name });
        move('next');
      } catch (thrown) {
        const error = toApiError(thrown);
        if (error.code === 'device.needs_firmware') set({ needsFirmware: true });
        else set({ error: { text: `Couldn't save to ${called()}. ${PLUG_IN}`, detail: error.message } });
      } finally {
        set({ busy: null });
      }
    },

    chooseOrientation(orientation: Orientation) {
      if (!state().busy) set({ orientation, error: null });
    },

    async submitOrientation(): Promise<void> {
      if (state().busy) return;
      const orientation = state().orientation;
      if (orientation === orientationOf(device)) {
        move('next');
        return readOnDevice();
      }
      set({ busy: 'saving', error: null });
      const before = settings.getState().notice;
      settings.chooseOrientation(orientation);
      await settings.confirmOrientation();
      if (settings.getState().restart === 'restarting') set({ busy: 'restarting' });
      const { notice } = await until(settings, (s) => s.restart !== 'restarting');
      set({ busy: null });
      if (notice !== before && notice?.kind === 'error') return set({ error: { text: notice.text, detail: notice.detail ?? '' } });
      move('next');
      return readOnDevice();
    },

    chooseLayout(choice: LayoutChoice) {
      if (!state().busy && (choice !== 'keep' || state().onDevice)) set({ choice, error: null });
    },

    async submitLayout(): Promise<void> {
      const { busy, choice, orientation, onDevice } = state();
      if (busy) return;
      if (choice === 'keep') {
        if (onDevice) keepDraft(onDevice.layout, onDevice.revision, false);
        return move('next');
      }
      const layout = choice === 'starter' ? starterLayout(orientation) : newLayout(orientation);
      set({ busy: 'sending', error: null });
      try {
        const job = await companion.layouts.push({ device: device.id, orientation }, null, layout);
        const report = await deps.wait(job);
        keepDraft(layout, report.revision, true);
        set({ onDevice: { layout, revision: report.revision } });
        move('next');
      } catch (thrown) {
        set({ error: { text: `Couldn't send to ${called()}. ${PLUG_IN}`, detail: toApiError(thrown).message } });
      } finally {
        set({ busy: null });
      }
    },

    offerFirmware() {
      set((s) => ({ flow: flowStep(s.flow, { type: 'offer', firmwareFirst: v1 }) }));
    },

    next() {
      if (!state().busy) move('next');
    },

    back() {
      if (state().busy) return;
      move('back');
      if (state().flow.at === 'layout') void readOnDevice();
    },

    observe(summary: DeviceSummary) {
      device = summary;
      if ((summary.firmware?.protocol ?? 1) >= 2 && state().needsFirmware) set({ needsFirmware: false });
    },
  };

  return Object.assign(store, api);
}

export type SetupStore = ReturnType<typeof createSetup>;

export function describeOnDevice(layout: Layout): string {
  const { pages, tiles } = counts(layout);
  return `The Layout on the Device now: ${pages} ${pages === 1 ? 'Page' : 'Pages'}, ${tiles} ${tiles === 1 ? 'Tile' : 'Tiles'}.`;
}
