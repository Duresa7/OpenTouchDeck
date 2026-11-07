import { afterEach, describe, expect, it, vi } from 'vitest';
import { createClient } from '../client.ts';
import { createFakeCompanion } from '../fake.ts';
import { keychainMissing, keychainName, platformOf } from '../keychain.ts';
import { windowSession } from '../session.ts';

afterEach(() => vi.unstubAllGlobals());

describe('the keychain', () => {
  it.each([
    ['windows', 'Windows Credential Manager', /^Windows Credential Manager isn't available\. Restart Open TouchDeck/],
    ['linux', 'your system keychain (GNOME Keyring or KWallet)', /^No system keychain is available\. Start GNOME Keyring or KWallet/],
    ['other', 'your system keychain', /^No system keychain is available\./],
    [null, 'your system keychain', /^No system keychain is available\./],
  ] as const)('on %s is %s', (platform, name, missing) => {
    expect(keychainName(platform)).toBe(name);
    expect(keychainMissing(platform)).toMatch(missing);
  });

  it('follows the platform the Companion reports, asked once per connection', async () => {
    const fake = createFakeCompanion({ system: { platform: 'linux' } });
    const asked: string[] = [];
    vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => {
      asked.push(new URL(input instanceof Request ? input.url : String(input)).pathname);
      return fake.fetch(input, init);
    });
    const companion = createClient('http://127.0.0.1:51234', windowSession(fake.bridge));
    expect(await platformOf(companion)).toBe('linux');
    expect(await platformOf(companion)).toBe('linux');
    expect(asked.filter((path) => path === '/api/v1/system')).toHaveLength(1);
  });

  it('asks again after the Companion couldn’t answer', async () => {
    const fake = createFakeCompanion({ system: { platform: 'windows' } });
    let down = true;
    vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => (down ? Promise.reject(new TypeError('Failed to fetch')) : fake.fetch(input, init)));
    const companion = createClient('http://127.0.0.1:51234', windowSession(fake.bridge));
    expect(await platformOf(companion)).toBeNull();
    down = false;
    expect(await platformOf(companion)).toBe('windows');
  });
});
