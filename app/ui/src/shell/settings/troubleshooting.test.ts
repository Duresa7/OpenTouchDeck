import { describe, expect, test } from 'vitest';
import type { LinkState } from '../../lib/companion/api.ts';
import { sampleDevice } from '../../lib/companion/samples.ts';
import { createUiStore } from '../../state/ui.ts';
import { connectionText, factsOf, lastError } from './troubleshooting.ts';

const connected: LinkState = { state: 'connected', port: 'COM7', since_ms: 0, protocol: 2 };

describe('Troubleshooting', () => {
  test('a connected Device shows the eight fields under the copy labels', () => {
    const device = sampleDevice('a0b1', 'Desk', connected);
    device.layout = { source: 'storage', name: 'Streaming', orientation: 'landscape', pages: 3, page: 1 };
    expect(factsOf(device).map((f) => [f.label, f.value])).toEqual([
      ['Product', 'touchdeck'],
      ['Protocol', '2'],
      ['Firmware build', 'fake'],
      ['Firmware version', '1.5.0'],
      ['Running from', 'ota_0'],
      ['Sends keys through', 'log, link'],
      ['Safe mode', 'No'],
      ['Current Layout', 'builtin'],
    ]);
  });

  test('without Firmware details or a Layout summary nothing is invented', () => {
    const device = { ...sampleDevice('a0b1', 'Desk', connected), firmware: null, diagnostics: null, layout: null };
    expect(factsOf(device).every((f) => f.value === null)).toBe(true);
  });

  test('control, bidi and separator characters a Device sends show as escapes', () => {
    const device = sampleDevice('a0b1', 'Desk', connected);
    device.firmware = { ...device.firmware!, build: 'ab\u0007cd', version: '1.4.0‏' };
    device.diagnostics = { product: 'touch\u0000deck', running_from: 'ota_0‮evil', sinks: ['log\n', 'li nk'], current_layout: 'a\u001B[31mb' };
    const shown = Object.fromEntries(factsOf(device).map((fact) => [fact.key, fact.value]));
    expect(shown).toMatchObject({
      product: 'touch\\u{0000}deck',
      elf_sha256: 'ab\\u{0007}cd',
      version: '1.4.0\\u{200F}',
      running: 'ota_0\\u{202E}evil',
      sinks: 'log\\u{000A}, li\\u{2028}nk',
      layout: 'a\\u{001B}[31mb',
    });
  });

  test('empty hello values remain known rather than becoming Unknown', () => {
    const device = sampleDevice('a0b1', 'Desk', connected);
    device.diagnostics = { product: '', running_from: '', sinks: [], current_layout: '' };
    expect(factsOf(device).filter((fact) => ['product', 'running', 'sinks', 'layout'].includes(fact.key)).map((fact) => fact.value)).toEqual(['', '', '', '']);
  });

  test.each<[LinkState, string]>([
    [connected, 'Desk is on COM7.'],
    [{ state: 'connecting', port: 'COM7' }, 'Connecting…'],
    [{ state: 'busy', port: 'COM7', detail: 'Access is denied.' }, 'Another program is using Desk. Close it, and Desk reconnects on its own.'],
    [{ state: 'unresponsive', port: 'COM7' }, "Desk didn't answer. Unplug it and plug it back in."],
    [{ state: 'lost', reason: 'timeout', detail: 'no reply within 1500 ms', at_ms: 1 }, 'Lost Desk. Check the USB cable. It reconnects on its own.'],
    [{ state: 'updating', job: 'job_1' }, 'Desk is updating. Wait for it to finish.'],
    [{ state: 'offline' }, "Desk isn't connected."],
    [{ state: 'disconnected' }, "Desk isn't connected."],
  ])('says where the Device is or what is wrong: %o', (link, text) => {
    expect(connectionText({ device: sampleDevice('a0b1', 'Desk', link), demo: false })).toBe(text);
  });

  test('the mock Device is the demo Device, and an unnamed one still reads as a sentence', () => {
    expect(connectionText({ device: sampleDevice('mock0001', 'Desk', connected), demo: true })).toBe('Desk is the demo Device.');
    expect(connectionText({ device: sampleDevice('a0b1', null, { state: 'offline' }), demo: false })).toBe("Unnamed Device isn't connected.");
  });

  test('a Device on Wi-Fi says so, with the address it was reached at', () => {
    const device = { ...sampleDevice('a0b1', 'Desk', { ...connected, port: '192.0.2.21:47820' }), carrier: 'wifi' as const };
    expect(connectionText({ device, demo: false })).toBe('Desk is on Wi-Fi at 192.0.2.21:47820.');
  });

  test('the raw link error is kept for Details only when there is one', () => {
    expect(lastError(sampleDevice('a', 'Desk', { state: 'lost', reason: 'io-error', detail: 'write failed: 31', at_ms: 1 }))).toBe('write failed: 31');
    expect(lastError(sampleDevice('a', 'Desk', { state: 'lost', reason: 'unplugged', detail: '', at_ms: 1 }))).toBeNull();
    expect(lastError(sampleDevice('a', 'Desk', connected))).toBeNull();
  });

  test('Settings opened from the nav starts on General, even after Connection help opened Troubleshooting', () => {
    const ui = createUiStore();
    ui.getState().openTroubleshooting();
    expect(ui.getState()).toMatchObject({ section: 'settings', settingsSection: 'troubleshooting' });
    ui.getState().go('home');
    expect(ui.getState().settingsSection).toBe('troubleshooting');
    ui.getState().openSettings();
    expect(ui.getState()).toMatchObject({ section: 'settings', settingsSection: 'general', settingsDevice: null });
    ui.getState().openSettings('device', 'two');
    expect(ui.getState()).toMatchObject({ settingsSection: 'device', settingsDevice: 'two' });
  });
});
