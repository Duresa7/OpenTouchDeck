import { describe, expect, it } from 'vitest';
import { bridgeReadyText, bridgeText, changeLines, checkText, downloadingText, iconLabel, needsWindows11Text, percent, sizeText, waitText } from './update-text.ts';

const MB = 1024 * 1024;

describe('changeLines', () => {
  it('turns Markdown release notes into plain change lines', () => {
    const notes = "## What's Changed\r\n\r\n- Volume Tiles follow your finger.\n* Devices reconnect faster.\n  + Nested <b>tags</b> stay text\nPlain line\n-\n";
    expect(changeLines(notes)).toEqual(['Volume Tiles follow your finger.', 'Devices reconnect faster.', 'Nested <b>tags</b> stay text', 'Plain line']);
  });

  it('gives no lines for missing or blank notes', () => {
    expect(changeLines(null)).toEqual([]);
    expect(changeLines('\n  \n## Only a heading')).toEqual([]);
  });
});

describe('progress', () => {
  it('rounds down, so 100% means every byte arrived', () => {
    expect(percent({ kind: 'downloading', downloaded: 999, total: 1000 })).toBe(99);
    expect(percent({ kind: 'downloading', downloaded: 1000, total: 1000 })).toBe(100);
    expect(percent({ kind: 'downloading', downloaded: 5, total: null })).toBeNull();
    expect(percent({ kind: 'downloading', downloaded: 5, total: 0 })).toBeNull();
  });

  it('says percent when the size is known, and KB when it is not', () => {
    expect(downloadingText({ kind: 'downloading', downloaded: 0.62 * 64e6, total: 64e6 })).toBe('Downloading… 62%');
    expect(downloadingText({ kind: 'downloading', downloaded: 300 * 1024, total: null })).toBe('Downloading… 300 KB');
  });

  it('shows megabytes with a decimal under 10 MB', () => {
    expect(sizeText({ kind: 'downloading', downloaded: 39.68e6, total: 64e6 })).toBe('38 MB of 61 MB');
    expect(sizeText({ kind: 'downloading', downloaded: 1.5 * MB, total: 8 * MB })).toBe('1.5 MB of 8.0 MB');
    expect(sizeText({ kind: 'downloading', downloaded: MB, total: null })).toBeNull();
  });
});

describe('labels', () => {
  it('names the icon for each step', () => {
    expect(iconLabel({ kind: 'offer' })).toBe('Update available');
    expect(iconLabel({ kind: 'failed', reason: 'signature' })).toBe('Update available');
    expect(iconLabel({ kind: 'downloading', downloaded: 50, total: 100 })).toBe('Downloading… 50%');
    for (const step of [{ kind: 'ready' }, { kind: 'busy', device: null }, { kind: 'installing' }] as const) {
      expect(iconLabel(step)).toBe('Ready to restart');
    }
  });

  it('names the Device a restart waits for', () => {
    expect(waitText('Desk')).toBe('Wait for Desk to finish sending.');
    expect(waitText(null)).toBe('Wait for the Device to finish sending.');
  });

  it('gives About a line for each check result', () => {
    expect(checkText('checking')).toBe('Checking…');
    expect(checkText('current')).toBe('Up to date');
    expect(checkText('offline')).toBe("You're offline. Connect to the internet, then retry.");
    expect(checkText('failed')).toBe("Can't reach the update server. Check your internet connection, then retry.");
    expect(checkText('found')).toBeNull();
    expect(checkText('idle')).toBeNull();
    expect(checkText('disabled')).toBeNull();
  });
});

describe('the move to 2.x', () => {
  it('says what changes and what stays', () => {
    expect(bridgeText({ version: '2.0.0', current: '1.3.0' })).toBe(
      'You have 1.3.0. Open TouchDeck 2.0.0 is the new app for Windows 11 and replaces this one. Layouts, settings, Devices and Pairings stay.',
    );
    expect(bridgeReadyText('2.0.0')).toBe('Open TouchDeck closes, installs 2.0.0 and opens it. Layouts, settings, Devices and Pairings stay.');
    expect(needsWindows11Text('2.0.0')).toBe('Open TouchDeck 2.0.0 needs Windows 11. This PC keeps Open TouchDeck 1.x, which still gets updates.');
  });
});
