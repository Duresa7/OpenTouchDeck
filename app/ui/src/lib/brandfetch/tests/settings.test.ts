import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Platform } from '../../companion/api.ts';
import { createClient } from '../../companion/client.ts';
import { ApiError } from '../../companion/errors.ts';
import { createFakeCompanion, type FakeCompanion, type FakeCompanionOptions } from '../../companion/fake.ts';
import { windowSession } from '../../companion/session.ts';
import { brandfetchSource, pickProblem, RECENT_NAME } from '../pick.ts';
import { clientIdOf, createBrandfetchStore, isBusy, logoCount, replacementsOf, type BrandfetchView } from '../settings.ts';

const BASE = 'http://127.0.0.1:51234';
let fake: FakeCompanion;

function use(options: FakeCompanionOptions = {}) {
  fake = createFakeCompanion(options);
  vi.stubGlobal('fetch', fake.fetch);
}

beforeEach(() => use());
afterEach(() => vi.unstubAllGlobals());

const store = () => createBrandfetchStore(createClient(BASE, windowSession(fake.bridge)));
const step = (view: BrandfetchView) => (view.kind === 'ready' ? view.step : null);

describe('Brandfetch settings', () => {
  it('turn search on with a client ID and keep the key write-only', async () => {
    const settings = store();
    await settings.getState().load();
    expect(clientIdOf(settings.getState().view)).toBeNull();
    expect(await settings.getState().saveClientId(' test-client-id ')).toBe(true);
    expect(clientIdOf(settings.getState().view)).toBe('test-client-id');
    expect(await settings.getState().saveKey('test-api-key')).toBe(true);
    const view = settings.getState().view;
    expect(view.kind === 'ready' && view.status.key_set).toBe(true);
    expect(step(view)).toEqual({ kind: 'idle' });
    expect(JSON.stringify(view)).not.toContain('test-api-key');
    expect(JSON.stringify(fake.brandfetch.status())).not.toContain('test-api-key');
    expect(await settings.getState().forgetClientId()).toBe(true);
    expect(clientIdOf(settings.getState().view)).toBeNull();
  });

  it('take one change at a time', async () => {
    const settings = store();
    await settings.getState().load();
    const first = settings.getState().saveClientId('test-client-id');
    expect(isBusy(settings.getState().view)).toBe(true);
    expect(await settings.getState().saveKey('test-api-key')).toBe(false);
    expect(await first).toBe(true);
    expect(isBusy(settings.getState().view)).toBe(false);
  });

  it('test the key only when asked, with one call', async () => {
    const settings = store();
    await settings.getState().load();
    expect(await settings.getState().testKey()).toBe(false);
    expect(step(settings.getState().view)).toEqual({ kind: 'failed', problem: { text: 'Save an API key first.', detail: 'Add your Brandfetch API key in Settings, then retry.' } });
    expect(await settings.getState().saveKey('test-api-key')).toBe(true);
    expect(fake.brandfetch.keyTests).toBe(0);
    expect(await settings.getState().testKey()).toBe(true);
    expect(fake.brandfetch.keyTests).toBe(1);
    expect(step(settings.getState().view)).toEqual({ kind: 'done', notice: 'Brandfetch accepted the key.' });
  });

  it.each([
    ['linux', 'No system keychain is available. Start GNOME Keyring or KWallet and try again.'],
    ['windows', "Windows Credential Manager isn't available. Restart Open TouchDeck and try again."],
  ] as const)('on %s, say how to start the keychain when there is none, with the Companion’s words as Details', async (platform: Platform, text) => {
    use({ system: { platform }, brandfetch: { keychain: false } });
    const settings = store();
    await settings.getState().load();
    expect(await settings.getState().saveKey('test-api-key')).toBe(false);
    const view = settings.getState().view;
    expect(step(view)).toEqual({ kind: 'failed', problem: { text, detail: "The system keychain isn't available." } });
    expect(view.kind === 'ready' && view.status.key_set).toBe(false);
  });

  it('explain a rejected client ID', async () => {
    const settings = store();
    await settings.getState().load();
    expect(await settings.getState().saveClientId('has space')).toBe(false);
    const failed = step(settings.getState().view);
    expect(failed?.kind === 'failed' && failed.problem.text).toMatch(/client ID/);
  });

  it('say the Companion is unreachable', async () => {
    vi.stubGlobal('fetch', () => Promise.reject(new TypeError('Failed to fetch')));
    const settings = store();
    await settings.getState().load();
    expect(settings.getState().view).toEqual({ kind: 'unreadable', problem: expect.stringMatching(/Can't reach the Companion/) });
  });

  it('read the replacements a refresh made', async () => {
    use({ brandfetch: { replaced: { ['a'.repeat(64)]: 'b'.repeat(64) } } });
    const settings = store();
    expect(replacementsOf(settings.getState().view)).toEqual({});
    await settings.getState().load();
    expect(replacementsOf(settings.getState().view)).toEqual({ ['a'.repeat(64)]: 'b'.repeat(64) });
  });

  it('count logos in plain words', () => {
    expect(logoCount({ logos: 0, need_refresh: 0 })).toBeNull();
    expect(logoCount({ logos: 1, need_refresh: 0 })).toBe('1 logo, all up to date');
    expect(logoCount({ logos: 3, need_refresh: 1 })).toBe('3 logos; 1 needs a refresh');
    expect(logoCount({ logos: 3, need_refresh: 2 })).toBe('3 logos; 2 need a refresh');
  });
});

describe('Brandfetch picks', () => {
  it('ask the Companion for the brand by domain and remember no brand data', () => {
    expect(brandfetchSource({ domain: 'music.example', brandId: 'id_fixture_music' })).toEqual({
      kind: 'brandfetch',
      brand: 'music.example',
      logo_type: 'icon',
      theme: 'auto',
      padding: 0,
      background: null,
      tint: null,
    });
    expect(brandfetchSource({ domain: '', brandId: 'id_fixture_music' }).brand).toBe('id_fixture_music');
    expect(RECENT_NAME).toBe('Brandfetch logo');
  });

  it.each([
    ['brandfetch.no_key', /API key in Settings/],
    ['brandfetch.key_refused', /refused the API key/],
    ['brandfetch.quota', /used up/],
    ['brandfetch.not_found', /no details for Example/],
    ['brandfetch.no_logo', /no logo for Example/],
    ['brandfetch.unavailable', /internet connection/],
    ['limits.exceeded', /still being prepared/],
  ] as const)('explain %s', (code, message) => {
    expect(pickProblem(new ApiError(code, 'x'), 'Example', 'windows')).toMatch(message);
  });

  it('word a missing keychain for the platform, never with the Companion’s text alone', () => {
    const error = new ApiError('brandfetch.no_keychain', "The system keychain isn't available.");
    expect(pickProblem(error, 'Example', 'linux')).toBe('No system keychain is available. Start GNOME Keyring or KWallet and try again.');
    expect(pickProblem(error, 'Example', 'windows')).toMatch(/^Windows Credential Manager isn't available/);
    expect(pickProblem(error, 'Example', null)).not.toBe(error.message);
  });
});
