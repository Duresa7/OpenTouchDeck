import clsx from 'clsx';
import type { Orientation } from '../lib/companion/api.ts';
import { geometry, screenSize } from '../lib/deck/geometry.ts';
import { buildDots, buildPage, buildTitle, FONTS, type FontRole, type TextView } from '../lib/deck/scene.ts';
import type { Line } from '../lib/deck/text.ts';
import { NO_LAYOUT, NO_LAYOUT_HINT, type Preview } from './home-model.ts';

const BOX_HEIGHT = 0.72;
const BOX_EDGE = 2;

function emptyBoxPath(x: number, y: number, w: number, h: number): string {
  const e = BOX_EDGE;
  return `M${x} ${y}h${w}v${h}h${-w}Z M${x + e} ${y + e}v${h - 2 * e}h${w - 2 * e}v${2 * e - h}Z`;
}

function SceneLine({ line, role }: { line: Line; role: FontRole }) {
  const font = FONTS[role];
  const height = Math.round(font.size * BOX_HEIGHT);
  const top = line.y + font.lineHeight - font.baseLine - height;
  const runs: { x: number; text: string }[] = [];
  const boxes: string[] = [];
  let run: { x: number; text: string } | null = null;
  for (const letter of line.letters) {
    if (letter.placeholder) {
      boxes.push(emptyBoxPath(line.x + letter.x + 1, top, letter.width - 2, height));
      run = null;
      continue;
    }
    if (!run) {
      run = { x: line.x + letter.x, text: '' };
      runs.push(run);
    }
    run.text += String.fromCodePoint(letter.letter);
  }
  return <>
    {runs.map((run, index) => <text key={index} className={`is-${role}`} x={run.x} y={line.y} dominantBaseline="hanging" fontSize={font.size}>{run.text}</text>)}
    {boxes.length > 0 && <path className="preview-box" d={boxes.join(' ')} fillRule="evenodd" />}
  </>;
}

function SceneText({ view }: { view: TextView }) {
  return <g transform={`translate(${view.x} ${view.y})`} opacity={view.opacity / 255}>
    {view.label.lines.map((line, index) => <SceneLine key={index} line={line} role={view.font} />)}
  </g>;
}

export function DevicePreview({ preview, live, orientation = 'landscape', empty = false }: { preview: Preview | null; live: boolean; orientation?: Orientation; empty?: boolean }) {
  const g = geometry(screenSize(preview?.layout.orientation ?? orientation), preview?.layout.fullScreen);
  const page = preview && buildPage(preview.layout.pages[preview.index], g);
  const title = page && buildTitle(page.title, g);
  return <span className={clsx('device-object', !live && 'is-off')} aria-hidden="true">
    <span className="device-bezel">
      <span className="device-screen" style={{ aspectRatio: `${g.screen.w} / ${g.screen.h}` }}>
        <svg className="device-scene" viewBox={`0 0 ${g.screen.w} ${g.screen.h}`} data-page={page?.id}>
          {title && <g className="preview-title"><SceneText view={title} /></g>}
          {page?.tiles.map((tile) => <svg key={tile.id} x={tile.rect.x} y={tile.rect.y} width={tile.rect.w} height={tile.rect.h} overflow="hidden">
            <rect width={tile.rect.w} height={tile.rect.h} rx={18} className="preview-tile" />
            {tile.texts.map((view, i) => !view.icon && <SceneText key={i} view={view} />)}
            {tile.bar && <rect {...{ x: tile.bar.x, y: tile.bar.y, width: tile.bar.w, height: tile.bar.h, rx: tile.bar.radius }} className="preview-active" />}
          </svg>)}
          {preview && buildDots(g, preview.layout.pages.length, preview.index).map((dot, i) => <rect key={i} x={dot.x} y={dot.y} width={dot.w} height={dot.h} rx={dot.h / 2} className={dot.active ? 'preview-active' : 'preview-idle'} />)}
        </svg>
        {empty && !preview && <span className="device-empty" data-testid="device-empty">
          <span className="device-empty-title">{NO_LAYOUT}</span>
          <span className="device-empty-hint">{NO_LAYOUT_HINT}</span>
        </span>}
      </span>
    </span>
  </span>;
}
