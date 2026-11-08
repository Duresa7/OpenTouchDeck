import { expect, test } from 'vitest';
import { createFakeIntegrations } from '../fake.ts';

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
