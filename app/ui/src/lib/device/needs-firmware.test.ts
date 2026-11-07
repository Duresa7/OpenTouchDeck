import { describe, expect, test } from 'vitest';
import type { Capabilities } from '../companion/api.ts';
import { sampleDevice } from '../companion/samples.ts';
import { compareVersions, needsFirmware, needsFirmwareField, newerFirmware, shortVersion, showsKind, tileKind } from './needs-firmware.ts';

const caps = (tile_kinds: string[]): Capabilities => ({
  screen: [1024, 600],
  grid: [4, 6],
  pages: 8,
  tiles: 48,
  layout_bytes: 65536,
  layout_schemas: [1],
  storage_free: 1_000_000,
  tile_kinds,
  asset_bytes: 1_000_000,
  formats: ['png'],
  image: [512, 512],
  live_hz: 0,
  live_total_hz: 0,
  series: 0,
  text_bytes: 0,
  live_images: 0,
  photo_bytes: 0,
});

const v1 = sampleDevice('mock0001', 'Desk');
const v2 = { ...v1, capabilities: caps(['static', 'clock']) };
const running = (version: string) => ({ firmware: { version, protocol: 2, build: 'fake', recovery: false, update_available: null, channel: 'normal' as const } });

describe('tile kinds', () => {
  test('a schema 1 Tile has no render, so it is static', () => {
    expect(tileKind({ id: 't1', label: 'Mute' })).toBe('static');
  });

  test('a schema 2 Tile is its render', () => {
    expect(tileKind({ id: 't1', render: 'clock' })).toBe('clock');
  });

  test('an unknown render reads as static', () => {
    expect(tileKind({ id: 't1', render: 'hologram' })).toBe('static');
  });

  test('v1 Firmware shows only static Tiles', () => {
    expect(showsKind(v1, 'static')).toBe(true);
    expect(showsKind(v1, 'clock')).toBe(false);
  });

  test('v2 Firmware shows the kinds its hello lists, whatever its version', () => {
    expect(showsKind(v2, 'clock')).toBe(true);
    expect(showsKind(v2, 'gif')).toBe(false);
  });
});

describe('versions', () => {
  test.each([
    ['1.5.0', '1.4.9', 1],
    ['1.4.0', '1.5.0', -1],
    ['1.4', '1.4.0', 0],
    ['v1.10.0', '1.9.0', 1],
  ])('%s against %s', (a, b, sign) => {
    expect(Math.sign(compareVersions(a, b) ?? Number.NaN)).toBe(sign);
  });

  test('a version it cannot read compares as nothing', () => {
    expect(compareVersions('nightly', '1.0.0')).toBeNull();
    expect(newerFirmware(running('nightly'), '1.5.0')).toBeNull();
  });

  test('the bundled build is offered only when it is newer', () => {
    expect(newerFirmware(running('1.4.0'), '1.5.0')).toBe('1.5.0');
    expect(newerFirmware(running('1.5.0'), '1.5.0')).toBeNull();
    expect(newerFirmware(running('1.6.0'), '1.5.0')).toBeNull();
    expect(newerFirmware({ firmware: null }, '1.5.0')).toBeNull();
    expect(newerFirmware(running('1.4.0'), null)).toBeNull();
  });

  test('the short version drops the patch', () => {
    expect(shortVersion('1.5.0')).toBe('1.5');
    expect(shortVersion('nightly')).toBe('nightly');
  });
});

describe('the Needs Firmware badge', () => {
  test('a kind the Device shows has no badge', () => {
    expect(needsFirmware(v1, 'static', '1.6.0')).toBeNull();
    expect(needsFirmware(v2, 'clock', '1.6.0')).toBeNull();
  });

  test('a kind the Device lacks names the newer Firmware', () => {
    expect(needsFirmware(v1, 'clock', '1.6.0')).toBe('Needs Firmware 1.6');
    expect(needsFirmware(v2, 'gif', '1.6.2')).toBe('Needs Firmware 1.6');
  });

  test('without a newer build the badge asks for newer Firmware', () => {
    expect(needsFirmware(v1, 'graph', null)).toBe('Needs newer Firmware');
  });

  test('a setting the Firmware lacks names the version that adds it, when there is one', () => {
    expect(needsFirmwareField({ device: 'Desk', since: '1.6.0' })).toBe('Needs Firmware 1.6.');
    expect(needsFirmwareField({ device: 'Desk', since: null })).toBe("Desk's Firmware can't change this yet.");
  });
});
