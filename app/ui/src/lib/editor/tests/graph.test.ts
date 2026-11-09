import assert from 'node:assert/strict';
import { test } from 'vitest';
import type { IntegrationSummary } from '../../companion/api.ts';
import { fakeMedia, fakeVolume } from '../../companion/fake.ts';
import { fakeThisPc } from '../../companion/fake.ts';
import { barFor, graphBinding, graphSources, placedLook, presetLive, sourceKey } from '../graph.ts';
import { graphTile, liveTile, newTile, parseLayout, type Layout, type Tile } from '../../layout/model.ts';

const thisPc = fakeThisPc().summary;

const hub: IntegrationSummary = {
  ...thisPc,
  id: 'hub',
  name: 'Hub',
  streams: [
    { id: 'title', kind: 'text', unit: null, settable: false },
    { id: 'five_hour_pct', kind: 'numeric', unit: '%', settable: false },
    { id: 'requests', kind: 'numeric', unit: null, settable: false },
  ],
  presets: [{ id: 'five_hour_pct', name: 'five hour pct', action: { type: 'widget', source: 'hub', field: 'five_hour_pct', account: '' } }],
};

test('the stream picker lists every numeric stream of every Integration, in order, named by its Preset tile', () => {
  const sources = graphSources([thisPc, hub]);
  assert.equal(sources.length, 12);
  assert.deepEqual(sources.slice(0, 2), [
    { integration: 'this_pc', stream: 'cpu_pct', integrationName: 'This PC', name: 'CPU usage', unit: '%' },
    { integration: 'this_pc', stream: 'ram_pct', integrationName: 'This PC', name: 'RAM usage', unit: '%' },
  ]);
  assert.deepEqual(
    sources.slice(10).map((s) => [s.integrationName, s.name, s.unit]),
    [
      ['Hub', 'Five hour pct', '%'],
      ['Hub', 'Requests', null],
    ],
  );
  assert.equal(new Set(sources.map(sourceKey)).size, sources.length);
});

test('with no numeric streams there is nothing to pick', () => {
  assert.deepEqual(graphSources([]), []);
  assert.deepEqual(graphSources([{ ...hub, streams: hub.streams.filter((s) => s.kind !== 'numeric') }]), []);
});

test('a graph Tile reads the Integration stream its widget action names', () => {
  const tile = graphTile(newTile('t', 0, 0), { integration: 'this_pc', stream: 'gpu_pct' });
  assert.deepEqual(graphBinding(tile), { integration: 'this_pc', stream: 'gpu_pct' });
  assert.equal(graphBinding(newTile('u', 0, 1)), null);
  assert.equal(graphBinding({ ...newTile('w', 0, 2), action: { type: 'widget', source: 'this_pc', field: 'cpu_pct', account: '' } }), null);
});

const GRAPH = { render: 'graph', streams: { value: 'number' } } as const;

test('a Preset becomes a graph only when its Integration offers it as one and its stream is numeric', () => {
  const cpu = thisPc.presets[0];
  assert.deepEqual(presetLive(cpu, thisPc), GRAPH);
  assert.equal(presetLive(hub.presets[0], hub), null);
  assert.equal(presetLive({ ...cpu, render: 'graph' }, hub), null);
  const text = { ...hub, streams: [{ id: 'cpu_pct', kind: 'text', unit: null, settable: false }], id: 'this_pc' };
  assert.equal(presetLive(cpu, text), null);
  assert.equal(presetLive({ ...cpu, action: { type: 'none' } }, thisPc), null);
});

const volume = fakeVolume().summary;
const media = fakeMedia().summary;
const [volumeBar] = volume.presets;
const [nowPlaying] = media.presets;

test('the Volume bar Preset is a draggable bar of the level with its muted look, and Now playing binds all four streams', () => {
  assert.deepEqual(presetLive(volumeBar, volume), { render: 'bar', streams: { value: 'number', muted: 'number' }, input: true });
  assert.deepEqual(presetLive(nowPlaying, media), { render: 'now-playing', streams: { title: 'text', artist: 'text', cover: 'image', state: 'number' } });
  assert.deepEqual(presetLive({ ...volumeBar, streams: { value: 'number' }, input: false }, volume), { render: 'bar', streams: { value: 'number' } });
  assert.deepEqual(presetLive({ ...volumeBar, streams: null }, volume), { render: 'bar', streams: { value: 'number' }, input: true });
});

test('a Preset whose streams its renderer cannot take, or that asks for input off a bar, stays plain', () => {
  const { cover: _cover, ...noCover } = nowPlaying.streams ?? {};
  assert.equal(presetLive({ ...nowPlaying, streams: noCover }, media), null);
  assert.equal(presetLive({ ...nowPlaying, streams: null }, media), null);
  assert.equal(presetLive({ ...nowPlaying, streams: { ...nowPlaying.streams, title: 'number' } }, media), null);
  assert.equal(presetLive({ ...volumeBar, streams: { value: 'number', level: 'number' } }, volume), null);
  assert.equal(presetLive({ ...volumeBar, streams: { value: 'series' } }, volume), null);
  assert.equal(presetLive({ ...nowPlaying, input: true }, media), null);
  assert.equal(presetLive(volumeBar, media), null);
});

test('without graph Firmware a graph Preset is placed plain, but a bar or now-playing Preset never is', () => {
  assert.deepEqual(placedLook(thisPc.presets[0], thisPc, true), GRAPH);
  assert.equal(placedLook(thisPc.presets[0], thisPc, false), null);
  assert.deepEqual(placedLook(volumeBar, volume, false), presetLive(volumeBar, volume));
  assert.deepEqual(placedLook(nowPlaying, media, false), presetLive(nowPlaying, media));
});

test('a Tile bound to a stream with a bar Preset can draw that bar, and other streams have none', () => {
  const level = { type: 'widget', source: 'volume', field: 'level', account: '' } as const;
  assert.deepEqual(barFor([thisPc, volume, media], level), presetLive(volumeBar, volume));
  assert.equal(barFor([thisPc, volume], { ...level, field: 'muted' }), null);
  assert.equal(barFor([thisPc], level), null);
  assert.equal(barFor([thisPc, volume], { ...level, source: 'this_pc', field: 'cpu_pct' }), null);
});

test('a Live tile from each Preset is a valid schema 2 Tile that keeps its action and shows no icon', () => {
  const cases = [
    [volumeBar, volume, 'level'],
    [nowPlaying, media, 'title'],
    [thisPc.presets[0], thisPc, 'cpu_pct'],
  ] as const;
  for (const [preset, integration, field] of cases) {
    const look = presetLive(preset, integration);
    assert.ok(look);
    const action = { type: 'widget', source: integration.id, field, account: '' } as const;
    const iconTile: Tile = { id: 't', label: 'T', row: 0, col: 0, rowSpan: 1, colSpan: 1, icon: 'lv:mute', action: { type: 'none' } };
    const tile = liveTile(iconTile, action, look);
    assert.equal(tile.icon, null);
    assert.deepEqual(tile.action, action);
    const layout: Layout = { schema: 2, name: 'L', orientation: 'landscape', pages: [{ kind: 'grid', id: 'p', title: 'P', rows: 2, cols: 2, tiles: [tile] }] };
    assert.deepEqual(parseLayout(JSON.stringify(layout)), { ok: true, layout });
  }
});
