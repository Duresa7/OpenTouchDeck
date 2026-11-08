import { useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import clsx from 'clsx';
import { Tooltip } from '@base-ui/react/tooltip';
import type { IntegrationSummary, PresetTile } from '../../lib/companion/api.ts';
import { presetLabel } from '../../lib/integrations/model.ts';
import { presetView, type PreviewOptions } from './add-tile.ts';
import { TileFace } from './TileScene.tsx';

/** Whether a clamped name lost its end, so the card shows it in full on hover. */
function useCut() {
  const ref = useRef<HTMLSpanElement>(null);
  const [cut, setCut] = useState(false);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => setCut(el.scrollHeight > el.clientHeight + 1);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  return [ref, cut] as const;
}

export interface PresetCardProps {
  preset: PresetTile;
  integration: Pick<IntegrationSummary, 'id' | 'streams'>;
  options: PreviewOptions;
  /** "Needs Firmware x.y" when the Device can't draw this Tile yet. */
  needs: string | null;
  onAdd(preset: PresetTile): void;
}

/**
 * A Preset tile as the Device will draw it, with example data, at the size a cell of the target Page has. The real
 * Tile renderer draws it in Device pixels and the card scales it to fit. Clicking it places it.
 */
export function PresetCard({ preset, integration, options, needs, onAdd }: PresetCardProps) {
  const { rows, cols, orientation, fullScreen, graphs, series } = options;
  const view = useMemo(
    () => presetView(preset, integration, { rows, cols, orientation, fullScreen, graphs, series }),
    [preset, integration, rows, cols, orientation, fullScreen, graphs, series],
  );
  const frame = useRef<HTMLSpanElement>(null);
  const [scale, setScale] = useState(0);
  const [nameRef, cut] = useCut();
  const width = view?.rect.w ?? 0;
  const label = presetLabel(preset.name);

  useLayoutEffect(() => {
    const el = frame.current;
    if (!el || !width) return;
    const fit = () => setScale(el.clientWidth / width);
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(el);
    return () => observer.disconnect();
  }, [width]);

  if (!view) return null;
  const screen: CSSProperties = { width: view.rect.w, height: view.rect.h, transform: `scale(${scale})` };
  return (
    <Tooltip.Root disabled={!cut}>
      <Tooltip.Trigger
        render={
          <button
            type="button"
            className={clsx('preset-card', needs !== null && 'needs-firmware')}
            aria-label={needs ? `Add ${label}. ${needs}` : `Add ${label}`}
            onClick={() => onAdd(preset)}
            data-testid="preset"
            data-preset={preset.id}
            data-needs={needs ?? undefined}
            data-look={view.graph ? 'graph' : view.level ? 'level' : view.nowPlaying ? 'now-playing' : view.bar ? 'bar' : 'text'}
          />
        }
      >
        <span className="preset-stage">
          <span ref={frame} className="preset-frame" style={{ aspectRatio: `${view.rect.w} / ${view.rect.h}` }}>
            <span className="preset-screen" style={screen} aria-hidden>
              <span className="tile preset-tile">
                <TileFace view={view} label={label} />
              </span>
            </span>
          </span>
          {needs && <span className="preset-needs">{needs}</span>}
        </span>
        <span ref={nameRef} className="preset-name" dir="auto">
          {label}
        </span>
      </Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Positioner className="tooltip-positioner" side="top" sideOffset={8}>
          <Tooltip.Popup className="tooltip">{label}</Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}
