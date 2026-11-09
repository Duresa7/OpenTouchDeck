import type { IntegrationSummary, PresetTile } from '../../companion/api.ts';
import { parseAction, streamLabel } from '../../integrations/model.ts';
import { RENDER_STREAMS, type BarStreams, type GraphBinding, type LiveLook, type Renderer, type StreamType, type Tile, type WidgetAction } from '../../layout/model.ts';

/** A numeric Integration stream a graph Tile can draw, named as its Preset tile when it has one. */
export interface GraphSource extends GraphBinding {
  integrationName: string;
  name: string;
  unit: string | null;
}

/** Every numeric stream of every Integration, in the Companion's order. */
export function graphSources(integrations: readonly IntegrationSummary[]): GraphSource[] {
  return integrations.flatMap((integration) =>
    integration.streams
      .filter((stream) => stream.kind === 'numeric')
      .map((stream) => ({
        integration: integration.id,
        stream: stream.id,
        integrationName: integration.name,
        name: streamLabel(integration, stream.id),
        unit: stream.unit,
      })),
  );
}

/** The stream a graph Tile draws, or null for any other Tile. */
export function graphBinding(tile: Tile): GraphBinding | null {
  if (tile.render !== 'graph' || tile.action.type !== 'widget') return null;
  return { integration: tile.action.source, stream: tile.action.field };
}

export const sourceKey = (binding: GraphBinding) => `${binding.integration}/${binding.stream}`;

const isRenderer = (render: unknown): render is Renderer => typeof render === 'string' && Object.hasOwn(RENDER_STREAMS, render);

function declaredStreams(render: Renderer, streams: PresetTile['streams']): Partial<Record<string, StreamType>> | null {
  const named: Partial<Record<string, StreamType>> = streams ?? { value: 'number' };
  const takes = RENDER_STREAMS[render];
  for (const [id, type] of Object.entries(named)) if (!type || !takes[id]?.types.includes(type)) return null;
  for (const [id, stream] of Object.entries(takes)) if (stream.required && !named[id]) return null;
  return named;
}

/** How a Preset tile draws as a Live tile: its renderer, the streams it declares and whether a drag sets it. Null for a plain Preset or one whose streams the renderer can't take. */
export function presetLive(preset: PresetTile, integration: Pick<IntegrationSummary, 'id' | 'streams'>): LiveLook | null {
  const action = parseAction(preset.action);
  if (!isRenderer(preset.render) || action?.type !== 'widget' || action.source !== integration.id) return null;
  if (preset.input && preset.render !== 'bar') return null;
  const streams = declaredStreams(preset.render, preset.streams);
  if (!streams) return null;
  switch (preset.render) {
    case 'graph': {
      const stream = integration.streams.find((s) => s.id === action.field);
      return stream?.kind === 'numeric' ? { render: 'graph', streams: { value: 'number' } } : null;
    }
    case 'bar': {
      const bar: BarStreams = streams.muted ? { value: 'number', muted: 'number' } : { value: 'number' };
      return preset.input ? { render: 'bar', streams: bar, input: true } : { render: 'bar', streams: bar };
    }
    case 'now-playing':
      return { render: 'now-playing', streams: { title: 'text', artist: 'text', cover: 'image', state: 'number' } };
  }
}

/**
 * How a Preset tile is placed on a Device: a graph only when the Device draws graphs, and a bar or now-playing Tile
 * always, because an interactive Preset is never turned into a plain value. The Tile then shows Needs Firmware.
 */
export function placedLook(preset: PresetTile, integration: Pick<IntegrationSummary, 'id' | 'streams'>, drawsGraphs: boolean): LiveLook | null {
  const look = presetLive(preset, integration);
  return look?.render === 'graph' && !drawsGraphs ? null : look;
}

/** The bar a Tile bound to this stream can draw: the look of its Integration's bar Preset for the stream, if it has one. */
export function barFor(integrations: readonly IntegrationSummary[], action: WidgetAction): LiveLook | null {
  const integration = integrations.find((i) => i.id === action.source);
  if (!integration) return null;
  for (const preset of integration.presets) {
    const bound = parseAction(preset.action);
    const look = presetLive(preset, integration);
    if (look?.render === 'bar' && bound?.type === 'widget' && bound.field === action.field) return look;
  }
  return null;
}
