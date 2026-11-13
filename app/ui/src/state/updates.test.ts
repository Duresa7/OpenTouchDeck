import { afterEach, describe, expect, it, vi } from 'vitest';
import { createFakeUpdates, sampleOffer, type FakeUpdatesScript } from '../lib/companion/fake.ts';
import { CHECK_EVERY_MS, createUpdatesStore, watchForUpdates, type UpdatesBridge, type UpdateStep } from './updates.ts';

function setup(script: Partial<FakeUpdatesScript> = {}) {
  const fake = createFakeUpdates({ stepMs: 0, check: sampleOffer, ...script });
  const store = createUpdatesStore(fake.bridge);
  return { fake, store, state: () => store.getState() };
}

async function offered(script: Partial<FakeUpdatesScript> = {}) {
  const ctx = setup(script);
  await ctx.state().checkForUpdates();
  return ctx;
}

function recordSteps(store: ReturnType<typeof createUpdatesStore>) {
  const steps: UpdateStep[] = [];
  store.subscribe((state) => {
    if (state.update && steps.at(-1) !== state.update.step) steps.push(state.update.step);
  });
  return steps;
}

afterEach(() => {
  vi.useRealTimers();
});

describe('without an updater, as in a browser tab', () => {
  it('shows nothing and never checks', async () => {
    const store = createUpdatesStore(undefined);
    expect(store.getState().supported).toBe(false);
    await store.getState().checkForUpdates();
    expect(store.getState()).toMatchObject({ check: 'idle', update: null });
    vi.useFakeTimers();
    watchForUpdates(store);
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe('checking', () => {
  it('reports checking, then offers the newer version', async () => {
    const { state } = setup();
    const running = state().checkForUpdates();
    expect(state().check).toBe('checking');
    await running;
    expect(state().check).toBe('found');
    expect(state().update).toEqual({ offer: sampleOffer, step: { kind: 'offer' } });
  });

  it('says up to date when there is nothing newer', async () => {
    const { state } = await offered({ check: { state: 'none' } });
    expect(state()).toMatchObject({ check: 'current', update: null });
  });

  it('shows nothing on a source build without a public key', async () => {
    const { state } = await offered({ check: { state: 'disabled' } });
    expect(state()).toMatchObject({ check: 'disabled', update: null });
  });

  it('shares one call between overlapping checks', async () => {
    const { fake, state } = setup();
    await Promise.all([state().checkForUpdates(), state().checkForUpdates()]);
    expect(fake.calls.check).toBe(1);
    await state().checkForUpdates();
    expect(fake.calls.check).toBe(2);
  });

  it('treats an updater that rejects as a failed check', async () => {
    const rejecting: UpdatesBridge = {
      check: () => Promise.reject(new Error('not available yet')),
      download: () => Promise.reject(new Error('not available yet')),
      install: () => Promise.reject(new Error('not available yet')),
    };
    const store = createUpdatesStore(rejecting);
    await store.getState().checkForUpdates();
    expect(store.getState()).toMatchObject({ check: 'failed', update: null });
  });

  it('keeps a known offer when a later check is offline or fails', async () => {
    const { fake, state } = await offered();
    for (const failure of ['offline', 'failed'] as const) {
      fake.script({ check: { state: failure } });
      await state().checkForUpdates();
      expect(state().check).toBe(failure);
      expect(state().update?.offer.version).toBe('1.1.0');
    }
  });

  it('replaces an offer with a newer one, but never a download in progress', async () => {
    const { fake, state } = await offered({ holdAt: 0.5 });
    fake.script({ check: { ...sampleOffer, version: '1.2.0' } });
    await state().checkForUpdates();
    expect(state().update).toMatchObject({ offer: { version: '1.2.0' }, step: { kind: 'offer' } });

    const downloading = state().download();
    await vi.waitFor(() => expect(state().update?.step).toMatchObject({ kind: 'downloading', downloaded: 32_000_000 }));
    fake.script({ check: { ...sampleOffer, version: '1.3.0' } });
    await state().checkForUpdates();
    expect(state().update).toMatchObject({ offer: { version: '1.2.0' }, step: { kind: 'downloading' } });
    fake.finish();
    await downloading;
    expect(state().update?.step).toEqual({ kind: 'ready' });
  });

  it('checks on start and every 6 hours', async () => {
    vi.useFakeTimers();
    const { fake, store } = setup();
    const stop = watchForUpdates(store);
    await vi.advanceTimersByTimeAsync(0);
    expect(fake.calls.check).toBe(1);
    await vi.advanceTimersByTimeAsync(CHECK_EVERY_MS - 1);
    expect(fake.calls.check).toBe(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(fake.calls.check).toBe(2);
    stop();
    await vi.advanceTimersByTimeAsync(CHECK_EVERY_MS * 2);
    expect(fake.calls.check).toBe(2);
  });
});

describe('downloading', () => {
  it('follows progress, then is ready to restart', async () => {
    const { store, state } = await offered({ size: 1000 });
    const steps = recordSteps(store);
    await state().download();
    expect(steps[0]).toEqual({ kind: 'downloading', downloaded: 0, total: null });
    expect(steps).toContainEqual({ kind: 'downloading', downloaded: 500, total: 1000 });
    expect(steps.at(-2)).toEqual({ kind: 'downloading', downloaded: 1000, total: 1000 });
    expect(steps.at(-1)).toEqual({ kind: 'ready' });
  });

  it('carries progress with no total when the size is unknown', async () => {
    const { store, state } = await offered({ size: null });
    const steps = recordSteps(store);
    await state().download();
    expect(steps.filter((step) => step.kind === 'downloading').every((step) => step.total === null)).toBe(true);
  });

  it.each([
    [{ state: 'failed', reason: 'signature' } as const, 'signature'],
    [{ state: 'failed', reason: 'download' } as const, 'download'],
    [{ state: 'offline' } as const, 'offline'],
  ])('a download that ends %o fails with %s, and Retry downloads again', async (outcome, reason) => {
    const { fake, state } = await offered({ download: outcome });
    await state().download();
    expect(state().update?.step).toEqual({ kind: 'failed', reason });
    fake.script({ download: { state: 'ready' } });
    await state().retry();
    expect(fake.calls.download).toBe(2);
    expect(state().update?.step).toEqual({ kind: 'ready' });
  });

  it('treats a rejected download as a failed download', async () => {
    const { fake, state } = await offered();
    fake.bridge.download = () => Promise.reject(new Error('lost'));
    await state().download();
    expect(state().update?.step).toEqual({ kind: 'failed', reason: 'download' });
  });

  it('starts nothing before an offer exists', async () => {
    const { fake, state } = setup();
    await state().download();
    await state().install();
    expect(fake.calls).toMatchObject({ download: 0, install: 0 });
  });
});

describe('installing', () => {
  it('waits for a busy Device, then reports an installer that would not start, and Retry installs again', async () => {
    const { fake, state } = await offered({ install: { state: 'failed', reason: 'busy', device: 'a0b1c2d3e4f5' } });
    await state().download();
    const restarting = state().install();
    expect(state().update?.step).toEqual({ kind: 'installing' });
    await restarting;
    expect(state().update?.step).toEqual({ kind: 'busy', device: 'a0b1c2d3e4f5' });

    fake.script({ install: { state: 'failed', reason: 'installer' } });
    await state().install();
    expect(state().update?.step).toEqual({ kind: 'failed', reason: 'installer' });

    await state().retry();
    expect(fake.calls).toMatchObject({ download: 1, install: 3 });
  });

  it('stays installing when the install succeeds, because the app quits', async () => {
    const { state } = await offered({ install: 'quit' });
    await state().download();
    void state().install();
    await new Promise((resolve) => setTimeout(resolve, 5));
    expect(state().update?.step).toEqual({ kind: 'installing' });
  });

  it('does not install before the download is ready', async () => {
    const { fake, state } = await offered();
    await state().install();
    expect(fake.calls.install).toBe(0);
    expect(state().update?.step).toEqual({ kind: 'offer' });
  });
});

describe('the dialog and Later', () => {
  it('opens only while an update exists', async () => {
    const { fake, state } = setup({ check: { state: 'none' } });
    state().setDialogOpen(true);
    expect(state().dialogOpen).toBe(false);
    fake.script({ check: sampleOffer });
    await state().checkForUpdates();
    state().setDialogOpen(true);
    expect(state().dialogOpen).toBe(true);
  });

  it('Later closes the dialog and quiets that version until a newer one is found', async () => {
    const { fake, state } = await offered();
    state().setDialogOpen(true);
    state().later();
    expect(state()).toMatchObject({ dialogOpen: false, laterFor: '1.1.0' });

    await state().checkForUpdates();
    expect(state().laterFor).toBe(state().update?.offer.version);

    fake.script({ check: { ...sampleOffer, version: '1.2.0' } });
    await state().checkForUpdates();
    expect(state().update?.offer.version).toBe('1.2.0');
    expect(state().laterFor).not.toBe('1.2.0');
  });
});

describe('the move to 2.x', () => {
  const bridgeOffer = { ...sampleOffer, version: '2.0.0', current: '1.3.0', bridge: true } as const;

  it('offers 2.x on Windows 11 like any update', async () => {
    const { state } = await offered({ check: bridgeOffer });
    expect(state().update).toEqual({ offer: bridgeOffer, step: { kind: 'offer' } });
    expect(state().needsWindows11).toBeNull();
  });

  it('remembers a 2.x that needs Windows 11 while 1.x is up to date, and forgets it when one is offered', async () => {
    const { fake, state } = await offered({ check: { state: 'none', needsWindows11: '2.0.0' } });
    expect(state()).toMatchObject({ check: 'current', update: null, needsWindows11: '2.0.0' });
    fake.script({ check: sampleOffer });
    await state().checkForUpdates();
    expect(state()).toMatchObject({ check: 'found', needsWindows11: null });
  });
});
