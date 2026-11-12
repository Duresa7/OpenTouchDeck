import type { CSSProperties, ReactNode } from 'react';
import clsx from 'clsx';
import { Film, Lock, Pause, Play, SkipBack, SkipForward, Volume2, VolumeX } from 'lucide-react';
import type { Rect } from '../../lib/deck/geometry.ts';
import { FONTS, type ControlGlyph, type GraphView, type LevelView, type NowPlayingView, type RowView, type TextView, type TileView } from '../../lib/deck/scene.ts';
import type { Line } from '../../lib/deck/text.ts';
import { THEME } from '../../lib/deck/theme.ts';
import { AssetImage } from './AssetImage.tsx';
import { GifImage } from './GifImage.tsx';
import { BuiltinIcon } from './glyphs.tsx';

// How the Device draws a Tile, in its own pixels. The Layout canvas and the Preset previews both draw with these, so
// a preview is the Tile that will be placed.

function runs(line: Line): ReactNode[] {
  const out: ReactNode[] = [];
  let text = '';
  for (const [i, letter] of line.letters.entries()) {
    if (!letter.placeholder) {
      text += String.fromCodePoint(letter.letter);
      continue;
    }
    if (text) out.push(text);
    text = '';
    out.push(<i key={i} className="scene-box" style={{ width: letter.width }} />);
  }
  if (text) out.push(text);
  return out;
}

export function SceneText({ view }: { view: TextView }) {
  const box: CSSProperties = { left: view.x, top: view.y, width: view.label.width, height: view.label.height };
  if (view.icon) {
    return (
      <span className="scene-icon" style={box}>
        <BuiltinIcon icon={view.icon} size={52} />
      </span>
    );
  }
  const lineHeight = FONTS[view.font].lineHeight;
  return (
    <span
      className={clsx('scene-text', `is-${view.font}`, view.color === THEME.color.text ? 'is-ink' : 'is-ink-2')}
      style={{ ...box, lineHeight: `${lineHeight}px`, opacity: view.opacity / 255 }}
      aria-hidden
    >
      {view.label.lines.map((line, i) => {
        const end = line.letters.at(-1);
        const width = end ? end.x + end.width : 0;
        const centred = line.x > 0 || width >= view.label.width - 1;
        const at: CSSProperties = centred ? { top: line.y, left: line.x + width / 2, translate: '-50% 0' } : { top: line.y, left: line.x };
        return (
          <span key={i} className="scene-line" style={at}>
            {runs(line)}
          </span>
        );
      })}
    </span>
  );
}

function SceneGraph({ view }: { view: GraphView }) {
  return (
    <span
      className="scene-graph"
      style={{
        left: view.x,
        top: view.y,
        width: view.w,
        height: view.h,
        borderRadius: view.radius,
        background: `color-mix(in srgb, var(--screen-accent) ${(THEME.graph.areaMix / 255) * 100}%, var(--screen-tile))`,
        opacity: view.stale ? THEME.staleOpa / 255 : 1,
      }}
      data-testid="tile-graph"
      data-points={view.points.length}
      data-stale={view.stale}
      aria-hidden
    >
      {view.points.length > 0 && (
        <svg width={view.w} height={view.h} viewBox={`0 0 ${view.w} ${view.h}`}>
          <polyline points={view.points.map((p) => `${p.x},${p.y}`).join(' ')} strokeWidth={THEME.graph.lineW} />
        </svg>
      )}
    </span>
  );
}

const at = (box: Rect): CSSProperties => ({ left: box.x, top: box.y, width: box.w, height: box.h });
const staleOpacity = (stale: boolean) => (stale ? THEME.staleOpa / 255 : 1);

function SceneLevel({ view }: { view: LevelView }) {
  const Speaker = view.glyph.muted ? VolumeX : Volume2;
  return (
    <span
      className={clsx('scene-live', view.muted && 'is-muted')}
      style={{ opacity: staleOpacity(view.stale) }}
      data-testid="tile-level"
      data-value={view.value.text}
      data-muted={view.muted}
      data-input={view.input}
      data-stale={view.stale}
      aria-hidden
    >
      <span className="scene-live-glyph" style={at(view.glyph)}>
        <Speaker size={view.glyph.w} strokeWidth={2.4} />
      </span>
      <span className="scene-level-label" style={at(view.label)}>
        {view.label.text}
      </span>
      <span className="scene-level-value" style={at(view.value)}>
        {view.value.text}
      </span>
      <span className="scene-level-track" style={{ ...at(view.track), borderRadius: view.track.radius }}>
        {view.fill > 0 && <span className="scene-level-fill" style={{ width: view.fill }} />}
      </span>
    </span>
  );
}

const CONTROL_SIZE = 32;

function ControlIcon({ glyph }: { glyph: ControlGlyph }) {
  const solid = { size: CONTROL_SIZE, strokeWidth: 0, fill: 'currentColor' };
  switch (glyph) {
    case 'prev':
      return <SkipBack {...solid} />;
    case 'play':
      return <Play {...solid} />;
    case 'pause':
      return <Pause {...solid} />;
    case 'next':
      return <SkipForward {...solid} />;
  }
}

function SceneNowPlaying({ view }: { view: NowPlayingView }) {
  const named = view.artist !== null;
  return (
    <span
      className="scene-live"
      style={{ opacity: staleOpacity(view.stale) }}
      data-testid="tile-now-playing"
      data-idle={view.idle}
      data-stale={view.stale}
      data-controls={view.controls.map((c) => c.glyph).join(' ')}
      aria-hidden
    >
      {view.art && (
        <span
          className={clsx('scene-np-art', named && 'is-stand-in')}
          style={{ left: view.art.x, top: view.art.y, width: view.art.size, height: view.art.size, borderRadius: THEME.nowPlaying.artRadius }}
          data-testid="tile-cover"
        />
      )}
      <span className={clsx('scene-np-title', view.title.idle && 'is-idle')} style={at(view.title)}>
        {view.title.text}
      </span>
      {view.artist && (
        <span className="scene-np-artist" style={at(view.artist)}>
          {view.artist.text}
        </span>
      )}
      {view.controls.map((control) => (
        <span key={control.part} className="scene-live-glyph" style={at(control)} data-part={control.part}>
          <ControlIcon glyph={control.glyph} />
        </span>
      ))}
    </span>
  );
}

export interface TileFaceProps {
  view: TileView;
  /** The Tile's label, shown in place of a Live tile the Device can't draw yet. */
  label: string;
  /** "Needs Firmware x.y" when the Device can't draw this Tile yet. */
  badge?: string | null;
  /** A modifier Tile that is locked on. */
  locked?: boolean;
}

/**
 * One Tile's face as the Device draws it, in Device pixels. It fills its parent, which gives it the Tile's size,
 * background and corners (`.tile`). It clips its contents to the Tile's corners, as the Device does.
 */
export function TileFace({ view, label, badge = null, locked = false }: TileFaceProps) {
  return (
    <span className="tile-face">
      {view.texts.map((text, n) => (
        <SceneText key={n} view={text} />
      ))}
      {view.image && (
        <span className="scene-image" style={{ left: view.image.x, top: view.image.y, width: view.image.size, height: view.image.size }}>
          <AssetImage key={view.image.id} id={view.image.id} size={view.image.size} />
        </span>
      )}
      {view.gif && (
        <span className="scene-gif" style={{ width: view.gif.w, height: view.gif.h }} data-testid="tile-gif">
          <GifImage key={view.gif.id} id={view.gif.id} missing={<Film className="scene-gif-missing" size={52} strokeWidth={1.6} data-testid="gif-missing" aria-hidden />} />
        </span>
      )}
      {view.bar && <span className="scene-bar" style={{ left: view.bar.x, top: view.bar.y, width: view.bar.w, height: view.bar.h, borderRadius: view.bar.radius }} />}
      {view.graph && <SceneGraph view={view.graph} />}
      {badge === null && view.level && <SceneLevel view={view.level} />}
      {badge === null && view.nowPlaying && <SceneNowPlaying view={view.nowPlaying} />}
      {badge !== null && (view.level || view.nowPlaying) && <span className="tile-needs-label">{label}</span>}
      {locked && <Lock className="scene-lock" size={30} strokeWidth={3} aria-hidden />}
      {badge !== null && (
        <span className="tile-needs" data-testid="tile-needs">
          {badge}
        </span>
      )}
    </span>
  );
}

/** A Usage Page's rows as the Device draws them, in Device pixels. Spans, so a Page thumbnail can sit in a button. */
export function SceneRows({ rows }: { rows: RowView[] }) {
  return rows.map((row, r) => (
    <span key={r} className="scene-row" style={{ left: row.rect.x, top: row.rect.y, width: row.rect.w, height: row.rect.h }}>
      {row.texts.map((text, n) => (
        <SceneText key={n} view={text} />
      ))}
      {row.bar && <span className="scene-bar" style={{ left: row.bar.x, top: row.bar.y, width: row.bar.w, height: row.bar.h, borderRadius: row.bar.radius }} />}
    </span>
  ));
}
