import { expect, test } from 'vitest';
import { createFakeIntegrations } from '../fake.ts';
import type { IntegrationView } from '../api.ts';

test('detail is null until a poll and carries typed account usage and named Preset tiles', async () => {
  const frames: unknown[] = [];
  let now = 1800000000000;
  const fake = createFakeIntegrations({ publish: frame => frames.push(frame), now: () => now });
  const call = async (method: string, path: string, body?: unknown): Promise<unknown> => {
    const request = new Request(`http://localhost/api/v1${path}`, { method, body: body === undefined ? undefined : JSON.stringify(body) });
    const response = await fake.handle(request, path);
    if (!response) throw new Error('Missing fake route');
    return response.status === 204 ? null : response.json();
  };
  const path = '/integrations/cliproxyapi';
  expect(await call('GET', path)).toHaveProperty('detail', null);
  await call('PUT', `${path}/config`, { enabled: true, url: 'https://hub.example', interval_s: 60 });
  await call('PUT', `${path}/secrets/management_key`, { value: 'fixture-private-value' });
  expect(await call('GET', path)).toHaveProperty('detail', null);
  await call('POST', `${path}/refresh`);
  const result = await call('GET', path);
  expect(result).toHaveProperty('detail.polled_ms', 1800000000000);
  expect(result).toHaveProperty('detail.accounts.0.account', 'Work');
  expect(result).toHaveProperty('detail.accounts.0.five_hour.resets_at_ms', 1800007800000);
  expect(result).toHaveProperty('detail.accounts.1.error', "The service refused this account's sign-in (HTTP 403). Sign in to it again in CLIProxyAPI.");
  expect(fake.list()[0].presets.map(p => p.name)).toEqual(['Account status', 'Requests OK', 'Requests failed', 'Requests 10 min', '5-hour used', '7-day used', '5-hour reset in', '7-day reset in', 'Plan']);
  expect(JSON.stringify([result, frames])).not.toContain('fixture-private-value');
  const revision = fake.list()[0].revision;
  expect(await call('PUT', `${path}/config`, { enabled: true, url: 'https://hub.example', interval_s: 60 })).toEqual(result);
  expect(fake.list()[0].revision).toBe(revision);
  fake.setHub(false);
  now += 15_000;
  await call('POST', `${path}/refresh`);
  expect(await call('GET', path)).toHaveProperty('detail.accounts.0.success', 128);
  expect(fake.list()[0].status.state).toBe('error');
  await call('DELETE', `${path}/secrets/management_key`);
  expect(await call('GET', path)).toHaveProperty('detail', null);
});

function fakeWithFrames() {
  const frames: { type: string; data?: { id?: string; enabled?: boolean } }[] = [];
  const fake = createFakeIntegrations({ publish: frame => frames.push(frame) });
  const send = async (method: string, path: string, body?: unknown) => {
    const response = await fake.handle(new Request(`http://localhost/api/v1${path}`, { method, body: body === undefined ? undefined : JSON.stringify(body) }), path);
    if (!response) throw new Error('Missing fake route');
    return response;
  };
  const call = async (method: string, path: string, body?: unknown): Promise<IntegrationView> => {
    const response = await send(method, path, body);
    expect(response.status, `${method} ${path}`).toBe(200);
    return response.json() as Promise<IntegrationView>;
  };
  const record = async () => (await fake.test(new Request('http://localhost/api/v1/test/integrations/record'), '/test/integrations/record').json()) as { settings: unknown; keySet: boolean; configPuts: number; enabledPuts: number };
  return { fake, frames, send, call, record };
}

test('enabled_put switches CLIProxyAPI and keeps its address, interval and key', async () => {
  const { frames, call, record } = fakeWithFrames();
  const path = '/integrations/cliproxyapi';
  await call('PUT', `${path}/config`, { enabled: true, url: 'https://hub.example', interval_s: 120 });
  await call('PUT', `${path}/secrets/management_key`, { value: 'fixture-private-value' });
  const configPuts = (await record()).configPuts;
  for (const enabled of [false, true]) {
    frames.length = 0;
    const view = await call('PUT', `${path}/enabled`, { enabled });
    expect(view.config).toEqual({ enabled, url: 'https://hub.example', interval_s: 120 });
    expect(view.summary.enabled).toBe(enabled);
    expect(view.secrets.management_key?.set).toBe(true);
    expect(frames).toContainEqual(expect.objectContaining({ type: 'integration.status', data: expect.objectContaining({ id: 'cliproxyapi', enabled }) }));
    expect(await call('GET', path)).toEqual(view);
  }
  frames.length = 0;
  await call('PUT', `${path}/enabled`, { enabled: true });
  expect(frames).toEqual([]);
  expect(await record()).toMatchObject({ keySet: true, configPuts, enabledPuts: 3 });
});

test('enabled_put switches the on-or-off Integrations', async () => {
  const { frames, call } = fakeWithFrames();
  for (const id of ['this_pc', 'volume', 'media']) {
    for (const enabled of [false, true]) {
      frames.length = 0;
      const view = await call('PUT', `/integrations/${id}/enabled`, { enabled });
      expect(view.config).toEqual({ enabled });
      expect(view.summary.enabled).toBe(enabled);
      expect(frames).toContainEqual(expect.objectContaining({ type: 'integration.status', data: expect.objectContaining({ id, enabled }) }));
      expect((await call('GET', `/integrations/${id}`)).config).toEqual({ enabled });
    }
  }
});

test('a config save without enabled keeps the stored value', async () => {
  const { call } = fakeWithFrames();
  const path = '/integrations/cliproxyapi';
  for (const enabled of [true, false]) {
    await call('PUT', `${path}/config`, { enabled, url: 'https://hub.example', interval_s: 120 });
    const view = await call('PUT', `${path}/config`, { url: 'https://hub.example/next', interval_s: 300 });
    expect(view.config).toEqual({ enabled, url: 'https://hub.example/next', interval_s: 300 });
  }
  for (const enabled of [false, true]) {
    await call('PUT', '/integrations/this_pc/enabled', { enabled });
    expect((await call('PUT', '/integrations/this_pc/config', {})).config).toEqual({ enabled });
  }
});

test('another client saving the form after a switch leaves CLIProxyAPI off', async () => {
  const { call, record } = fakeWithFrames();
  const path = '/integrations/cliproxyapi';
  await call('PUT', `${path}/config`, { enabled: true, url: 'https://hub.example', interval_s: 120 });
  await call('PUT', `${path}/secrets/management_key`, { value: 'fixture-private-value' });
  await call('PUT', `${path}/enabled`, { enabled: false });
  const saved = await call('PUT', `${path}/config`, { url: 'https://hub.example', interval_s: 300 });
  expect(saved.summary.enabled).toBe(false);
  expect(await record()).toMatchObject({ settings: { enabled: false, url: 'https://hub.example', interval_s: 300 }, keySet: true });
});

test('enabled_put takes only {enabled} and PUT, and needs a known Integration', async () => {
  const { send, record } = fakeWithFrames();
  for (const id of ['cliproxyapi', 'this_pc']) {
    for (const body of [{ enabled: 'yes' }, { enabled: true, url: 'https://hub.example' }, {}, [true], true]) {
      const response = await send('PUT', `/integrations/${id}/enabled`, body);
      expect(response.status).toBe(400);
      expect((await response.json()).error.code).toBe('request.invalid');
    }
    for (const method of ['GET', 'POST', 'DELETE']) {
      const response = await send(method, `/integrations/${id}/enabled`, method === 'GET' ? undefined : { enabled: true });
      expect(response.status).toBe(405);
      expect((await response.json()).error.code).toBe('route.method_not_allowed');
    }
  }
  const missing = await send('PUT', '/integrations/missing/enabled', { enabled: true });
  expect(missing.status).toBe(404);
  expect((await missing.json()).error.code).toBe('integration.not_found');
  expect(await record()).toMatchObject({ settings: { enabled: false }, enabledPuts: 0 });
});
