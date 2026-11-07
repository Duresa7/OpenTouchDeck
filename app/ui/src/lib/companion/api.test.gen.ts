// Generated from td-companion's test hooks. Do not edit.
export type ApprovalScript = { answers: Array<ScriptAnswer>, };
export type BoardKind = "device" | "device-v1" | "device-radio" | "silent" | "other-firmware" | "busy";
export type ClockAdvance = { advance_ms: number, };
export type ConsentAnswer = "accept" | "decline";
export type ConsentScript = { answers: Array<ConsentAnswer>, };
export type ConsoleReply = { 
/**
 * The `@td` JSON whose `cmd` is the line's first word, or null when none came in time.
 */
reply: JsonValue | null, 
/**
 * The Device's console lines from the send until the reply or the deadline, at most 256.
 */
lines: Array<string>, };
export type Delivery = { url: string | null, notification: string | null, };
export type DeviceId = string;
export type DevicePath = { device: DeviceId, };
export type JsonValue = number | string | boolean | Array<JsonValue> | { [key in string]?: JsonValue } | null;
export type MediaFixture = { title: string, artist: string, state: number, cover: string | null, };
export type MediaScriptBody = { session: MediaFixture | null, };
export type MockAction = { 
/**
 * `no-confirm`, `rollback`, `normal-update`, `stop` or `hang-up`.
 */
action: string, };
export type MockEmit = { events: Array<JsonValue>, repeat: number | null, every_ms: number | null, };
export type MockPortChange = { "plug": PlugBoard } | { "unplug": DeviceId };
export type MockPress = { page: string, tile: string, kind: string, };
export type MockRecord = { received: Array<JsonValue>, lines: Array<string>, settings: JsonValue, reboots: number, assets: Array<string>, layouts: JsonValue, };
export type NoQuery = null;
export type PlugBoard = { id: DeviceId, kind: BoardKind, };
export type ScriptAnswer = "once" | "always" | "deny" | "fail" | "never";
export type TestConsoleLine = { line: string, 
/**
 * Milliseconds to wait for the `@td` reply: 3000 when absent, 0 to 15000. With 0 the
 * route returns what arrived while it sent the line.
 */
wait_ms: number | null, };
export type VolumeInput = { level: number, muted: boolean, };
export const testEndpoints = {
  'test.approvals_script': { method: 'POST', path: '/test/approvals/script', access: 'session' },
  'test.clock': { method: 'POST', path: '/test/clock', access: 'session' },
  'test.console': { method: 'POST', path: '/test/devices/{device}/console', access: 'session' },
  'test.emit': { method: 'POST', path: '/test/devices/{device}/emit', access: 'session' },
  'test.firmware_script': { method: 'POST', path: '/test/firmware/script', access: 'session' },
  'test.input_log': { method: 'GET', path: '/test/input-log', access: 'session' },
  'test.media': { method: 'POST', path: '/test-hooks/media', access: 'session' },
  'test.mock': { method: 'POST', path: '/test/devices/{device}/mock', access: 'session' },
  'test.pairing_delivery': { method: 'GET', path: '/test/pairing-delivery', access: 'session' },
  'test.ports': { method: 'POST', path: '/test/ports', access: 'session' },
  'test.press': { method: 'POST', path: '/test/devices/{device}/press', access: 'session' },
  'test.record': { method: 'GET', path: '/test/devices/{device}/record', access: 'session' },
  'test.sets': { method: 'GET', path: '/test/devices/{device}/sets', access: 'session' },
  'test.volume_read': { method: 'GET', path: '/test-hooks/volume', access: 'session' },
  'test.volume_write': { method: 'POST', path: '/test-hooks/volume', access: 'session' },
} as const;
