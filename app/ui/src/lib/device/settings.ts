import { createStore } from 'zustand/vanilla';
import type { DeviceSettings, DeviceSettingsPatch, DeviceSummary, Orientation, PhotoFrameSettings, SettingsPatched } from '../companion/api.ts';
import type { Companion } from '../companion/client.ts';
import { toApiError, type ApiError } from '../companion/errors.ts';
import { hasHiddenCharacter } from './name-rule.ts';

export type Field = 'name' | 'orientation' | 'brightness' | 'photo_frame';
export type Unsupported = { since: string | null };
type Values = { name: string; orientation: Orientation; brightness: number; photo_frame: PhotoFrameSettings };
export type PhotoFrameChange = Partial<Pick<PhotoFrameSettings, 'enabled' | 'timeout_s' | 'folder'>>;

export interface Notice {
  kind: 'info' | 'ok' | 'error';
  text: string;
  detail?: string;
  event: string;
}

export interface DeviceSettingsState {
  load: 'idle' | 'loading' | 'ready' | 'failed';
  shown: DeviceSettings | null;
  saved: DeviceSettings | null;
  needs: Partial<Record<'name' | 'brightness' | 'photo_frame', Unsupported>>;
  problem: Partial<Record<'name' | 'folder', string>>;
  confirm: Orientation | null;
  restart: 'idle' | 'restarting' | 'failed';
  notice: Notice | null;
}

export interface DeviceSettingsDeps {
  companion: Pick<Companion, 'devices'>;
  device: Pick<DeviceSummary, 'id' | 'name' | 'firmware' | 'link'>;
  throttleMs?: number;
  restartTimeoutMs?: number;
}

export const NAME_BYTES = 48;

export type NameBreak = 'empty' | 'too-long' | 'special';

export function nameBreak(name: string): NameBreak | null {
  if (!/\S/u.test(name)) return 'empty';
  if (new TextEncoder().encode(name).length > NAME_BYTES) return 'too-long';
  if (hasHiddenCharacter(name)) return 'special';
  return null;
}

export const SPECIAL_CHARACTERS = 'Remove tabs, line breaks and other special characters.';

export function nameProblem(name: string): string | null {
  const broken = nameBreak(name);
  switch (broken) {
    case null:
      return null;
    case 'empty':
      return 'Give the Device a name.';
    case 'too-long':
      return 'That name is too long for the Device.';
    case 'special':
      return SPECIAL_CHARACTERS;
    default: {
      const _exhaustive: never = broken;
      return _exhaustive;
    }
  }
}

const NO_CHANGE: DeviceSettingsPatch = { idle_timeout_s: null, orientation: null, name: null, brightness: null, photo_frame: null };
const FIELDS = ['name', 'orientation', 'brightness', 'photo_frame'] as const;
const FOLDER_GONE = "That folder isn't on this PC anymore. Choose another.";

const detailsReason = (error: ApiError): string =>
  typeof error.details === 'object' && error.details !== null && 'reason' in error.details && typeof error.details.reason === 'string'
    ? error.details.reason
    : '';

const detailsVersion = (error: ApiError): string | null =>
  typeof error.details === 'object' && error.details !== null && 'version' in error.details && typeof error.details.version === 'string'
    ? error.details.version
    : null;

export function createDeviceSettings(deps: DeviceSettingsDeps) {
  const { companion, throttleMs = 150, restartTimeoutMs = 30_000 } = deps;
  let device = deps.device;
  const store = createStore<DeviceSettingsState>()(() => ({
    load: 'idle',
    shown: null,
    saved: null,
    needs: {},
    problem: {},
    confirm: null,
    restart: 'idle',
    notice: null,
  }));
  const { getState: state, setState: set } = store;
  const name = () => device.name ?? 'the Device';
  const say = (notice: Notice) => set({ notice });

  let seq = 0;
  let slide: number | null = null;
  const latest: Partial<Record<Field, number>> = {};
  const settled: Partial<Record<Field, number>> = {};
  const pending = (field: Field) => latest[field] !== settled[field] || (field === 'brightness' && slide !== null);

  function supported(settings: DeviceSettings): DeviceSettingsState['needs'] {
    const needs: DeviceSettingsState['needs'] = {};
    if ((device.firmware?.protocol ?? 1) < 2) needs.name = { since: null };
    if (settings.brightness === null) needs.brightness = { since: null };
    if (settings.photo_frame === null) needs.photo_frame = { since: null };
    return needs;
  }

  async function patch<F extends Field>(field: F, value: Values[F], { quiet = false, sent = {} }: { quiet?: boolean; sent?: Partial<DeviceSettingsPatch> } = {}): Promise<SettingsPatched | null> {
    const mine = ++seq;
    latest[field] = mine;
    set((s) => ({ shown: s.shown && { ...s.shown, [field]: value }, notice: quiet ? s.notice : null }));
    try {
      const reply = await companion.devices.settings_patch({ device: device.id }, null, { ...NO_CHANGE, [field]: value, ...sent });
      settled[field] = Math.max(settled[field] ?? 0, mine);
      set((s) => {
        const shown = { ...reply.settings };
        for (const key of FIELDS) if (pending(key) && s.shown) Object.assign(shown, { [key]: s.shown[key] });
        return { saved: reply.settings, shown };
      });
      if (reply.restart_required && field !== 'orientation') say({ kind: 'info', text: `Saved. Restart ${name()} to apply it.`, event: 'restart-required' });
      else if (!quiet) say({ kind: 'ok', text: 'Saved', event: 'saved' });
      return reply;
    } catch (thrown) {
      const error = toApiError(thrown);
      if (latest[field] === mine) settled[field] = mine;
      set((s) => ({ shown: s.shown && s.saved && latest[field] === mine ? { ...s.shown, [field]: s.saved[field] } : s.shown }));
      if (error.code === 'device.needs_firmware' && (field === 'name' || field === 'brightness' || field === 'photo_frame')) {
        set((s) => ({ needs: { ...s.needs, [field]: { since: detailsVersion(error) } } }));
      } else if (field === 'photo_frame' && error.code === 'request.invalid' && detailsReason(error).includes('folder')) {
        set((s) => ({ problem: { ...s.problem, folder: FOLDER_GONE } }));
      } else {
        say({ kind: 'error', text: `Couldn't save to ${name()}. Check that it's plugged in, then retry.`, event: 'save-failed', detail: error.message });
      }
      return null;
    }
  }

  let sending = false;
  let sent: number | null = null;
  let sentAt = -Infinity;
  let slideTimer: ReturnType<typeof setTimeout> | null = null;

  async function pump() {
    if (sending || slide === null) return;
    const wait = sentAt + throttleMs - Date.now();
    if (wait > 0) {
      slideTimer ??= setTimeout(() => {
        slideTimer = null;
        void pump();
      }, wait);
      return;
    }
    const value = slide;
    slide = null;
    sending = true;
    sent = value;
    sentAt = Date.now();
    const reply = await patch('brightness', value, { quiet: true });
    sending = false;
    if (reply === null) slide = sent = null;
    else void pump();
  }

  let restartTimer: ReturnType<typeof setTimeout> | null = null;
  let dropped = false;

  function restarted(failed: boolean) {
    if (restartTimer) clearTimeout(restartTimer);
    restartTimer = null;
    if (failed) {
      set({ restart: 'failed' });
      say({ kind: 'error', text: `Couldn't restart ${name()}. Unplug it and plug it back in.`, event: 'restart-failed' });
      return;
    }
    set({ restart: 'idle', notice: null });
    void api.load();
  }

  const api = {
    async load(): Promise<void> {
      if (device.link.state !== 'connected') return;
      set({ load: 'loading' });
      try {
        const settings = await companion.devices.settings_get({ device: device.id });
        set({ load: 'ready', saved: settings, shown: settings, needs: supported(settings) });
      } catch (thrown) {
        set({ load: 'failed' });
        say({ kind: 'error', text: `Couldn't read ${name()}'s settings. Unplug it and plug it back in.`, event: 'read-failed', detail: toApiError(thrown).message });
      }
    },

    async rename(next: string): Promise<boolean> {
      const shown = state().shown;
      if (!shown || state().needs.name) return false;
      const problem = nameProblem(next);
      set((s) => ({ problem: { ...s.problem, name: problem ?? undefined } }));
      if (problem) return false;
      if (next === shown.name) return true;
      return (await patch('name', next)) !== null;
    },

    chooseOrientation(orientation: Orientation) {
      set({ confirm: orientation === state().shown?.orientation ? null : orientation });
    },

    cancelOrientation() {
      set({ confirm: null });
    },

    async confirmOrientation(): Promise<void> {
      const orientation = state().confirm;
      if (!orientation) return;
      set({ confirm: null });
      const reply = await patch('orientation', orientation, { quiet: true });
      if (reply?.restart_required) await api.restart();
      else if (reply) say({ kind: 'ok', text: 'Saved', event: 'saved' });
    },

    async setPhotoFrame(change: PhotoFrameChange): Promise<boolean> {
      const current = state().shown?.photo_frame;
      if (!current || state().needs.photo_frame) return false;
      set((s) => ({ problem: { ...s.problem, folder: undefined } }));
      const photo_frame = { enabled: null, timeout_s: null, interval_s: null, folder: null, ...change };
      return (await patch('photo_frame', { ...current, ...change }, { sent: { photo_frame } })) !== null;
    },

    slideBrightness(value: number) {
      set((s) => ({ shown: s.shown && { ...s.shown, brightness: value } }));
      slide = value;
      void pump();
    },

    commitBrightness(value: number) {
      if (slide !== null || value !== sent) api.slideBrightness(value);
    },

    async restart(): Promise<void> {
      if (state().restart === 'restarting') return;
      dropped = false;
      set({ restart: 'restarting' });
      say({ kind: 'info', text: 'Restarting…', event: 'restarting' });
      try {
        await companion.devices.restart({ device: device.id });
      } catch (thrown) {
        set({ restart: 'failed' });
        say({ kind: 'error', text: `Couldn't restart ${name()}. Unplug it and plug it back in.`, event: 'restart-failed', detail: toApiError(thrown).message });
        return;
      }
      if (state().restart !== 'restarting') return;
      restartTimer = setTimeout(() => {
        if (dropped) restarted(true);
        else restarted(false);
      }, restartTimeoutMs);
    },

    observe(summary: Pick<DeviceSummary, 'id' | 'name' | 'firmware' | 'link'>) {
      device = summary;
      if (state().restart !== 'restarting') return;
      if (summary.link.state !== 'connected') dropped = true;
      else if (dropped) restarted(false);
    },

    dispose() {
      if (slideTimer) clearTimeout(slideTimer);
      if (restartTimer) clearTimeout(restartTimer);
    },
  };

  return Object.assign(store, api);
}

export type DeviceSettingsStore = ReturnType<typeof createDeviceSettings>;
