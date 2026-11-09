import type { IntegrationSummary, PresetTile } from '../../lib/companion/api.ts';
import { geometry, screenSize } from '../../lib/deck/geometry.ts';
import { buildPage, sampleScene, type TileView } from '../../lib/deck/scene.ts';
import { placedLook } from '../../lib/editor/graph.ts';
import { presetTile } from '../../lib/editor/store.ts';
import { offersPresets } from '../../lib/integrations/model.ts';
import { newGridPage, type GridPage, type Orientation } from '../../lib/layout/model.ts';

export type AddTab = 'actions' | 'integrations';

/** Add Tile opens on Integrations when one offers Preset tiles, unless the tab was chosen by hand this session. */
export function addTab(integrations: readonly IntegrationSummary[], chosen: AddTab | null): AddTab {
  return chosen ?? (integrations.some(offersPresets) ? 'integrations' : 'actions');
}

/** The grid a preview is drawn for: the Page Add Tile targets, or a new Page's grid when it starts one. */
export function previewGrid(page: { kind: string; rows?: number; cols?: number } | undefined, newPage: boolean): { rows: number; cols: number } {
  if (!newPage && page?.kind === 'grid' && page.rows && page.cols) return { rows: page.rows, cols: page.cols };
  const fresh = newGridPage('preview', '');
  return { rows: fresh.rows, cols: fresh.cols };
}

export interface PreviewOptions {
  rows: number;
  cols: number;
  orientation: Orientation;
  fullScreen: boolean;
  /** Whether the Device draws graphs; a graph Preset is previewed as the text Tile it would be placed as. */
  graphs: boolean;
  /** How many samples the Device's graphs keep. */
  series: number;
}

/**
 * The Tile a Preset places, drawn as the Device would draw it on a 1×1 cell of this grid with example data. Built the
 * way placement builds it, so the preview can't drift from what lands on the Page. Null when the Preset is refused.
 */
export function presetView(preset: PresetTile, integration: Pick<IntegrationSummary, 'id' | 'streams'>, options: PreviewOptions): TileView | null {
  const tile = presetTile(preset, placedLook(preset, integration, options.graphs));
  if (!tile) return null;
  const page: GridPage = { id: 'preview', title: '', kind: 'grid', rows: options.rows, cols: options.cols, tiles: [tile] };
  const g = geometry(screenSize(options.orientation), options.fullScreen);
  return buildPage(page, g, sampleScene(options.series, false)).tiles[0] ?? null;
}
