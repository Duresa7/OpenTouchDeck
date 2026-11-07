import { hasHiddenCharacter } from '../../device/name-rule.ts';
import type { DeviceSettings, DeviceSummary, LinkState, PhotoFrameSettings } from '../api.ts';

type Frame = Record<string, unknown> & { type: string };

export interface FakeDeviceSettingsOptions {
  devices: Map<string, DeviceSummary>;
  publish(frame: Frame): void;
  brightness?: Record<string, number>;
  photoFrame?: readonly string[];
  restartMs?: number;
}

const json = (status: number, body: unknown) =>
  new Response(status === 204 ? null : JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

const failure = (status: number, code: string, message: string, details: unknown = null) => json(status, { error: { code, message, details } });

const validName = (name: string) =>
  name.length > 0 && new TextEncoder().encode(name).length <= 48 && /\S/u.test(name) && !hasHiddenCharacter(name);

interface Stored {
  idle_timeout_s: number;
  orientation: 'landscape' | 'portrait';
  brightness: number | null;
  photo_frame: PhotoFrameSettings | null;
}

const between = (value: unknown, min: number, max: number) => typeof value === 'number' && Number.isInteger(value) && value >= min && value <= max;

const looksLikeFullPath = (folder: string) => /^[A-Za-z]:[\\/]/u.test(folder) || folder.startsWith('/') || folder.startsWith('\\\\');

export function createFakeDeviceSettings(options: FakeDeviceSettingsOptions) {
  const { devices, publish, restartMs = 300 } = options;
  const stored = new Map<string, Stored>();
  const reboots = new Map<string, number>();

  function settingsOf(device: DeviceSummary): Stored {
    let found = stored.get(device.id);
    if (!found) {
      found = {
        idle_timeout_s: 600,
        orientation: device.orientation === 'portrait' ? 'portrait' : 'landscape',
        brightness: options.brightness?.[device.id] ?? null,
        photo_frame: options.photoFrame?.includes(device.id) ? { enabled: false, timeout_s: 60, interval_s: 15, folder: null } : null,
      };
      stored.set(device.id, found);
    }
    return found;
  }

  const v2 = (device: DeviceSummary) => (device.firmware?.protocol ?? 1) >= 2;

  const view = (device: DeviceSummary): DeviceSettings => {
    const settings = settingsOf(device);
    return {
      idle_timeout_s: settings.idle_timeout_s,
      orientation: settings.orientation,
      name: v2(device) ? device.name : null,
      brightness: settings.brightness,
      photo_frame: settings.photo_frame && { ...settings.photo_frame },
    };
  };

  function setDevice(id: string, change: Partial<DeviceSummary>) {
    const current = devices.get(id);
    if (!current) return;
    const next = { ...current, ...change, rev: current.rev + 1 };
    devices.set(id, next);
    publish({ type: 'device.updated', device: id, data: next });
  }

  function reachable(id: string): DeviceSummary | Response {
    const device = devices.get(id);
    if (!device) return failure(404, 'device.not_found', 'No Device with that id.');
    if (device.link.state !== 'connected') return failure(409, 'device.not_connected', 'The Device is not connected.', { link: device.link });
    return device;
  }

  async function patch(request: Request, device: DeviceSummary): Promise<Response> {
    const body: unknown = await request.json().catch(() => null);
    const field = (name: string): unknown => (typeof body === 'object' && body !== null && name in body ? Reflect.get(body, name) : null);
    const invalid = (reason: string) => failure(400, 'request.invalid', 'The request body is invalid.', { part: 'body', reason });
    const orientation = field('orientation');
    const idle = field('idle_timeout_s');
    const name = field('name');
    const brightness = field('brightness');
    const frame = field('photo_frame');
    const part = (name: string): unknown => (typeof frame === 'object' && frame !== null && name in frame ? Reflect.get(frame, name) : null);
    const enabled = part('enabled');
    const timeout = part('timeout_s');
    const interval = part('interval_s');
    const folder = part('folder');
    if (orientation !== null && orientation !== 'landscape' && orientation !== 'portrait') return invalid('orientation is landscape or portrait');
    if (idle !== null && (typeof idle !== 'number' || !Number.isInteger(idle) || idle < 1 || idle > 86400)) return invalid('idle_timeout_s is out of range');
    if (name !== null && (typeof name !== 'string' || !validName(name))) return invalid('name is empty, too long or has control, format or filler characters');
    if (brightness !== null && (typeof brightness !== 'number' || !Number.isInteger(brightness) || brightness < 0 || brightness > 100)) return invalid('brightness is a percentage from 0 to 100');
    if (frame !== null && typeof frame !== 'object') return invalid('photo_frame is an object');
    if (enabled !== null && typeof enabled !== 'boolean') return invalid('photo_frame.enabled is true or false');
    if (timeout !== null && !between(timeout, 10, 86400)) return invalid('photo_frame.timeout_s is 10 to 86400');
    if (interval !== null && !between(interval, 5, 3600)) return invalid('photo_frame.interval_s is 5 to 3600');
    if (folder !== null && (typeof folder !== 'string' || (folder !== '' && !looksLikeFullPath(folder)))) {
      return invalid('photo_frame.folder is not the full path of a folder on this computer');
    }
    const settings = settingsOf(device);
    if (frame !== null && settings.photo_frame === null) return failure(409, 'device.needs_firmware', 'This needs newer Firmware on the Device.', { field: 'photo_frame', version: null });
    if (brightness !== null && settings.brightness === null) return failure(409, 'device.needs_firmware', 'This needs newer Firmware on the Device.', { field: 'brightness', version: null });
    if (name !== null && !v2(device)) return failure(409, 'device.needs_firmware', 'This needs newer Firmware on the Device.', { field: 'name', version: null });
    const restart = orientation !== null && orientation !== settings.orientation;
    if (orientation !== null) settings.orientation = orientation;
    if (typeof idle === 'number') settings.idle_timeout_s = idle;
    if (typeof brightness === 'number') settings.brightness = brightness;
    if (settings.photo_frame) {
      const photo = settings.photo_frame;
      if (typeof enabled === 'boolean') photo.enabled = enabled;
      if (typeof timeout === 'number') photo.timeout_s = timeout;
      if (typeof interval === 'number') photo.interval_s = interval;
      if (typeof folder === 'string') photo.folder = folder === '' ? null : folder;
    }
    if (typeof name === 'string' && (name !== device.name || device.setup !== 'ready')) setDevice(device.id, { name, setup: 'ready' });
    return json(200, { settings: view(devices.get(device.id) ?? device), restart_required: restart });
  }

  function restart(device: DeviceSummary & { link: Extract<LinkState, { state: 'connected' }> }) {
    const port = device.link.port;
    reboots.set(device.id, (reboots.get(device.id) ?? 0) + 1);
    setTimeout(() => setDevice(device.id, { link: { state: 'connecting', port } }), 0);
    setTimeout(() => setDevice(device.id, { link: { state: 'connected', port, since_ms: Date.now(), protocol: device.firmware?.protocol ?? 1 }, orientation: settingsOf(device).orientation }), restartMs);
  }

  return {
    async handle(request: Request, path: string): Promise<Response | null> {
      let match: RegExpExecArray | null;
      if ((match = /^\/devices\/([^/]+)\/settings$/.exec(path))) {
        const device = reachable(decodeURIComponent(match[1]));
        if (device instanceof Response) return device;
        if (request.method === 'GET') return json(200, view(device));
        if (request.method === 'PATCH') return patch(request, device);
        return failure(405, 'route.method_not_allowed', 'Use GET or PATCH.');
      }
      if ((match = /^\/devices\/([^/]+)\/restart$/.exec(path))) {
        if (request.method !== 'POST') return failure(405, 'route.method_not_allowed', 'Use POST.');
        const device = reachable(decodeURIComponent(match[1]));
        if (device instanceof Response) return device;
        if (device.link.state === 'connected') restart({ ...device, link: device.link });
        return json(204, null);
      }
      return null;
    },

    async record(path: string, answer: Response): Promise<Response> {
      const match = /^\/test\/devices\/([^/]+)\/record$/.exec(path);
      const device = match ? devices.get(decodeURIComponent(match[1])) : undefined;
      if (!device || !answer.ok) return answer;
      const body: unknown = await answer.json();
      return json(200, { ...(typeof body === 'object' && body !== null ? body : {}), settings: settingsOf(device), reboots: reboots.get(device.id) ?? 0 });
    },
  };
}
