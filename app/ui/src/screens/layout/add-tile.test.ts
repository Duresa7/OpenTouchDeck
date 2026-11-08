import { describe, expect, it } from 'vitest';
import type { IntegrationSummary } from '../../lib/companion/api.ts';
import { addTab, closingLine, missingPresets, presetView, previewGrid } from './add-tile.ts';

const preset = (id: string, render?: 'graph' | 'bar' | 'now-playing') => ({ id, name: id === 'cpu_pct' ? 'CPU usage' : id, action: { type: 'widget', source: 'this_pc', field: id, account: '' }, ...(render ? { render } : {}) });

function integration(id: string, name: string, enabled: boolean, state = 'ok', presets = [preset('cpu_pct', 'graph')]): IntegrationSummary {
  return { revision: 0, id, name, icon: '', enabled, status: { state, message: null, delivery: [] }, config_schema: [], streams: [{ id: 'cpu_pct', kind: 'numeric', unit: '%', settable: false }], actions: [], presets };
}

describe('Add Tile', () => {
  it('opens on Integrations when one offers Preset tiles, and keeps a tab chosen by hand', () => {
    expect(addTab([integration('this_pc', 'This PC', true)], null)).toBe('integrations');
    expect(addTab([integration('this_pc', 'This PC', false)], null)).toBe('actions');
    expect(addTab([integration('cliproxyapi', 'CLIProxyAPI', true, 'not_configured')], null)).toBe('actions');
    expect(addTab([integration('this_pc', 'This PC', true)], 'actions')).toBe('actions');
    expect(addTab([], 'integrations')).toBe('integrations');
  });

  it('closes the Integrations tab by naming what is off and what needs setup', () => {
    const list = [
      integration('this_pc', 'This PC', true),
      integration('volume', 'Volume', false),
      integration('media', 'Media', false),
      integration('cliproxyapi', 'CLIProxyAPI', true, 'not_configured'),
      integration('extra', 'Extra', false, 'ok', []),
    ];
    const missing = missingPresets(list);
    expect(missing.off.map((i) => i.id)).toEqual(['volume', 'media']);
    expect(missing.setUp.map((i) => i.id)).toEqual(['cliproxyapi']);
    expect(closingLine(missing)).toBe('Volume and Media are off. CLIProxyAPI needs setup.');
    expect(closingLine(missingPresets([integration('volume', 'Volume', false)]))).toBe('Volume is off.');
    expect(closingLine(missingPresets([integration('a', 'A', false), integration('b', 'B', false), integration('c', 'C', false)]))).toBe('A, B and C are off.');
    expect(closingLine(missingPresets([integration('this_pc', 'This PC', true)]))).toBeNull();
  });

  it("previews on the target Page's grid, or a new Page's when Add Tile starts one", () => {
    expect(previewGrid({ kind: 'grid', rows: 2, cols: 3 }, false)).toEqual({ rows: 2, cols: 3 });
    expect(previewGrid({ kind: 'grid', rows: 2, cols: 3 }, true)).toEqual({ rows: 3, cols: 4 });
    expect(previewGrid({ kind: 'dashboard' }, false)).toEqual({ rows: 3, cols: 4 });
    expect(previewGrid(undefined, false)).toEqual({ rows: 3, cols: 4 });
  });

  it('draws a graph Preset as a graph when the Device draws graphs, and as the text Tile it would place otherwise', () => {
    const pc = integration('this_pc', 'This PC', true);
    const options = { rows: 3, cols: 4, orientation: 'landscape' as const, fullScreen: false, series: 32 };
    const graph = presetView(pc.presets[0], pc, { ...options, graphs: true });
    expect(graph?.graph?.points.length).toBeGreaterThan(1);
    const text = presetView(pc.presets[0], pc, { ...options, graphs: false });
    expect(text?.graph).toBeNull();
    expect(text?.texts.length).toBeGreaterThan(0);
  });

  it('draws a Preset at the size one cell of the target grid has', () => {
    const pc = integration('this_pc', 'This PC', true);
    const options = { orientation: 'landscape' as const, fullScreen: false, series: 32, graphs: true };
    const small = presetView(pc.presets[0], pc, { ...options, rows: 3, cols: 4 })!;
    const big = presetView(pc.presets[0], pc, { ...options, rows: 2, cols: 3 })!;
    expect(big.rect.w).toBeGreaterThan(small.rect.w);
    expect(big.rect.h).toBeGreaterThan(small.rect.h);
  });

  it('draws nothing for a Preset whose action the Layout rules refuse', () => {
    const pc = integration('this_pc', 'This PC', true);
    const bad = { ...pc.presets[0], action: { type: 'widget', source: '', field: 'plan', account: '' } };
    expect(presetView(bad, pc, { rows: 3, cols: 4, orientation: 'landscape', fullScreen: false, series: 32, graphs: true })).toBeNull();
  });
});
