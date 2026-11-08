import { describe, expect, it } from 'vitest';
import type { IntegrationSummary, IntegrationView } from '../companion/api.ts';
import { createIntegrationsStore } from '../companion/integrations.ts';
import { completesPending, createWrites, flipOf, hasSettings, switchView } from './switch.ts';

const status = (state: string) => ({ status: { state, message: null, delivery: [] } });

describe('the Integration switch', () => {
  it('turns on what is set up, opens setup for what is not, and always turns off', () => {
    expect(flipOf(status('ok'), true)).toBe('enable');
    expect(flipOf(status('error'), true)).toBe('enable');
    expect(flipOf(status('not_configured'), true)).toBe('set-up');
    expect(flipOf(status('not_configured'), false)).toBe('disable');
    expect(flipOf(status('ok'), false)).toBe('disable');
  });

  it('shows the saved state, the state it is saving, or on while an enable waits for setup', () => {
    expect(switchView(true, null, false)).toEqual({ checked: true, busy: false, locked: false });
    expect(switchView(false, null, false)).toEqual({ checked: false, busy: false, locked: false });
    expect(switchView(true, false, false)).toEqual({ checked: false, busy: true, locked: true });
    expect(switchView(false, true, false)).toEqual({ checked: true, busy: true, locked: true });
    expect(switchView(true, null, true)).toEqual({ checked: true, busy: false, locked: false });
    expect(switchView(true, null, false, 1)).toEqual({ checked: true, busy: true, locked: true });
  });

  it('locks only while a write runs or waits, so an enable waiting for setup can be flipped off', () => {
    expect(switchView(false, null, true)).toEqual({ checked: true, busy: true, locked: false });
    expect(switchView(false, null, true, 1)).toEqual({ checked: true, busy: true, locked: true });
  });

  it('completes a pending enable once a save sets the Integration up', () => {
    expect(completesPending({ enabled: false, ...status('not_configured') }, true)).toBe(false);
    expect(completesPending({ enabled: false, ...status('ok') }, true)).toBe(true);
    expect(completesPending({ enabled: false, ...status('error') }, true)).toBe(true);
    expect(completesPending({ enabled: true, ...status('ok') }, true)).toBe(false);
    expect(completesPending({ enabled: false, ...status('ok') }, false)).toBe(false);
  });

  it('gives a settings page only to an Integration with config beyond its switch', () => {
    expect(hasSettings({ config_schema: [{ name: 'enabled', kind: 'boolean', secret: false }] })).toBe(false);
    expect(hasSettings({ config_schema: [{ name: 'enabled', kind: 'boolean', secret: false }, { name: 'url', kind: 'string', secret: false }] })).toBe(true);
  });
});

function summary(id: string, enabled: boolean): IntegrationSummary {
  return { revision: 0, id, name: id, icon: '', enabled, ...status('ok'), config_schema: [], streams: [], actions: [], presets: [] };
}

describe('Integration writes', () => {
  it('run one at a time per Integration, in order, and share one busy count', async () => {
    const order: string[] = [];
    let release!: () => void;
    const gate = new Promise<void>((resolve) => (release = resolve));
    const writes = createWrites({ integrations: { enabled_put: async () => ({}) as IntegrationView } } as never, createIntegrationsStore());
    const first = writes.run('a', async () => {
      await gate;
      order.push('first');
    });
    const second = writes.run('a', async () => void order.push('second'));
    const other = writes.run('b', async () => void order.push('other'));
    await other;
    expect(writes.getState().writes).toEqual({ a: 2 });
    release();
    await Promise.all([first, second]);
    expect(order).toEqual(['other', 'first', 'second']);
    expect(writes.getState().writes).toEqual({});
  });

  it('keep the queue going after a write fails', async () => {
    const writes = createWrites({ integrations: {} } as never, createIntegrationsStore());
    await expect(writes.run('a', async () => Promise.reject(new Error('nope')))).rejects.toThrow('nope');
    await expect(writes.run('a', async () => 'ok')).resolves.toBe('ok');
  });

  it('turn an Integration on through the narrow enable call and take the summary it answers', async () => {
    const calls: unknown[] = [];
    const store = createIntegrationsStore();
    store.getState().sync([summary('volume', false)]);
    const companion = {
      integrations: {
        enabled_put: async (path: unknown, query: unknown, body: unknown) => {
          calls.push([path, query, body]);
          return { summary: summary('volume', true), config: {}, secrets: {}, detail: null } satisfies IntegrationView;
        },
      },
    };
    const writes = createWrites(companion as never, store);
    const pending = writes.setEnabled('volume', true);
    expect(writes.getState().saving).toEqual({ volume: true });
    expect(await pending).not.toBeNull();
    expect(calls).toEqual([[{ integration: 'volume' }, null, { enabled: true }]]);
    expect(store.getState().integrations[0].enabled).toBe(true);
    expect(writes.getState()).toEqual({ saving: {}, writes: {} });
  });

  it('answer null when the Companion refuses, leaving the summary as it was', async () => {
    const store = createIntegrationsStore();
    store.getState().sync([summary('volume', true)]);
    const writes = createWrites({ integrations: { enabled_put: async () => Promise.reject(new Error('down')) } } as never, store);
    expect(await writes.setEnabled('volume', false)).toBeNull();
    expect(store.getState().integrations[0].enabled).toBe(true);
    expect(writes.getState().saving).toEqual({});
  });
});
