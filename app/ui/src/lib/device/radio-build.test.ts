import { describe, expect, test } from 'vitest';
import type { DeviceSummary, FirmwareBuild, FirmwareChannel } from '../companion/api.ts';
import { sampleDevice } from '../companion/samples.ts';
import { buildChoice, bundledVersion, channelOf, offeredFor, offersWifi } from './radio-build.ts';

const BOTH: FirmwareBuild = { version: '1.6.0', build: 'normal', radio: { version: '1.6.1', build: 'radio' } };
const NORMAL_ONLY: FirmwareBuild = { version: '1.6.0', build: 'normal', radio: null };

function running(channel: FirmwareChannel, version = '1.5.0'): Pick<DeviceSummary, 'firmware'> {
  const device = sampleDevice('mock0001', 'Desk');
  return { firmware: device.firmware && { ...device.firmware, version, channel } };
}

describe('the build a Device runs', () => {
  test('is the one its Firmware reports, and normal when nothing is reported', () => {
    expect(channelOf(running('radio'))).toBe('radio');
    expect(channelOf(running('normal'))).toBe('normal');
    expect(channelOf({ firmware: null })).toBe('normal');
  });

  test('only a Device on the radio build gets the Wi-Fi step', () => {
    expect(offersWifi(running('radio'))).toBe(true);
    expect(offersWifi(running('normal'))).toBe(false);
    expect(offersWifi({ firmware: null })).toBe(false);
  });
});

describe('updates stay on the build the Device runs', () => {
  test('each channel reads its own bundled version', () => {
    expect(bundledVersion(BOTH, 'normal')).toBe('1.6.0');
    expect(bundledVersion(BOTH, 'radio')).toBe('1.6.1');
    expect(bundledVersion(NORMAL_ONLY, 'radio')).toBeNull();
    expect(bundledVersion(null, 'normal')).toBeNull();
  });

  test('a radio Device is offered the radio build, never the normal one', () => {
    expect(offeredFor(running('radio'), BOTH)).toBe('1.6.1');
    expect(offeredFor(running('normal'), BOTH)).toBe('1.6.0');
    expect(offeredFor(running('radio'), NORMAL_ONLY)).toBeNull();
    expect(offeredFor(running('radio', '1.6.1'), BOTH)).toBeNull();
  });
});

describe('the switch', () => {
  test('from the normal build offers the radio build and says what it turns on', () => {
    const choice = buildChoice(running('normal'), BOTH, 'Desk');
    expect(choice).toMatchObject({ current: 'normal', target: 'radio', title: 'Normal build', action: 'Install the radio build', missing: null, warn: false });
    expect(choice?.note).toBe('The radio build adds Wi-Fi and Bluetooth, both off until you set them up. You can switch back any time.');
    expect(choice?.confirmTitle).toBe('Install the radio build on Desk?');
    expect(choice?.confirmNote).toBe('It adds Wi-Fi and Bluetooth, both off until you set them up. Desk restarts once.');
  });

  test('from the radio build switches back and warns the setup stays stored but unused', () => {
    const choice = buildChoice(running('radio'), BOTH, 'Desk');
    expect(choice).toMatchObject({ current: 'radio', target: 'normal', title: 'Radio build', action: 'Switch to the normal build', warn: true });
    expect(choice?.confirmTitle).toBe('Switch Desk back to the normal build?');
    expect(choice?.confirmNote).toBe('The Wi-Fi and Bluetooth setup stays stored on Desk but unused. Desk restarts once.');
  });

  test('is hidden from a normal Device when this Companion carries no radio build', () => {
    expect(buildChoice(running('normal'), NORMAL_ONLY, 'Desk')).toBeNull();
    expect(buildChoice(running('normal'), null, 'Desk')).toBeNull();
  });

  test('still shows a radio Device its build when this Companion carries no normal build', () => {
    const choice = buildChoice(running('radio'), null, 'Desk');
    expect(choice).toMatchObject({ current: 'radio', action: null, missing: "This copy of Open TouchDeck doesn't include the normal build." });
  });
});
