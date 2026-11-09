// PROTOTYPE — throwaway, answers: how should the Layout editor lay out so the nav never shifts?
// Pieces every variant may use: the deck state, the Device drawing, the nav rail, the Page list and small fields.
// Layout belongs to each variant, so nothing here decides where things sit on the screen.
import { useEffect, useRef, useState, type ComponentType, type KeyboardEvent } from 'react';
import clsx from 'clsx';
import {
  Cable,
  Calculator,
  Camera,
  Check,
  ClipboardPaste,
  Copy,
  Cpu,
  Folder,
  Gauge,
  Globe,
  HardDrive,
  House,
  Lamp,
  LayoutGrid,
  Lightbulb,
  LoaderCircle,
  Lock,
  MemoryStick,
  Mic,
  Moon,
  Music,
  Pencil,
  Play,
  Plus,
  Power,
  Settings,
  SkipBack,
  SkipForward,
  Sun,
  Terminal,
  Thermometer,
  Upload,
  Volume1,
  Volume2,
  VolumeX,
  Wifi,
  type LucideProps,
} from 'lucide-react';
import { ACTIONS, DEVICE_NAME, newId, PAGES, type IconKey, type Page, type Tile } from './layout-editor.prototype-data.ts';

const ICONS: Record<IconKey, ComponentType<LucideProps>> = {
  play: Play,
  back: SkipBack,
  next: SkipForward,
  mute: VolumeX,
  volUp: Volume2,
  volDown: Volume1,
  music: Music,
  mic: Mic,
  copy: Copy,
  paste: ClipboardPaste,
  camera: Camera,
  lock: Lock,
  terminal: Terminal,
  globe: Globe,
  calc: Calculator,
  folder: Folder,
  cpu: Cpu,
  memory: MemoryStick,
  gauge: Gauge,
  wifi: Wifi,
  disk: HardDrive,
  temp: Thermometer,
  lamp: Lamp,
  bulb: Lightbulb,
  sun: Sun,
  moon: Moon,
  power: Power,
};

export function Icon({ k, size = 18 }: { k: IconKey; size?: number }) {
  const C = ICONS[k];
  return <C size={size} strokeWidth={1.9} aria-hidden />;
}

export type Screen = 'home' | 'layout' | 'integrations' | 'settings';
export type Orientation = 'landscape' | 'portrait';
export type SendState = 'hidden' | 'unsent' | 'sending' | 'saved';

export interface Deck {
  screen: Screen;
  setScreen(screen: Screen): void;
  deviceName: string;
  rename(name: string): void;
  pages: Page[];
  page: Page;
  selectPage(id: string): void;
  addPage(): void;
  renamePage(title: string): void;
  setGrid(rows: number, cols: number): void;
  tile: Tile | null;
  selectTile(id: string | null): void;
  addTile(cell?: number): void;
  updateTile(patch: Partial<Tile>): void;
  deleteTile(): void;
  orientation: Orientation;
  setOrientation(o: Orientation): void;
  preview: boolean;
  setPreview(on: boolean): void;
  send: SendState;
  sendNow(): void;
}

/** Keeps each Tile where it was by row and column; a Tile that falls off moves to a free cell, or goes. */
function regrid(page: Page, rows: number, cols: number): Tile[] {
  const used = new Set<number>();
  const kept: Tile[] = [];
  const homeless: Tile[] = [];
  for (const t of page.tiles) {
    const r = Math.floor(t.cell / page.cols);
    const c = t.cell % page.cols;
    if (r < rows && c < cols) {
      const cell = r * cols + c;
      used.add(cell);
      kept.push({ ...t, cell });
    } else homeless.push(t);
  }
  for (const t of homeless) {
    let free = 0;
    while (used.has(free) && free < rows * cols) free++;
    if (free >= rows * cols) break;
    used.add(free);
    kept.push({ ...t, cell: free });
  }
  return kept;
}

export function useDeck(): Deck {
  const [screen, setScreen] = useState<Screen>('layout');
  const [deviceName, setDeviceName] = useState(DEVICE_NAME);
  const [pages, setPages] = useState<Page[]>(PAGES);
  const [pageId, setPageId] = useState(PAGES[0]!.id);
  const [tileId, setTileId] = useState<string | null>(null);
  const [orientation, setOrientation] = useState<Orientation>('landscape');
  const [preview, setPreviewState] = useState(false);
  const [send, setSend] = useState<SendState>('hidden');
  const timers = useRef<number[]>([]);

  const page = pages.find((p) => p.id === pageId) ?? pages[0]!;
  const tile = page.tiles.find((t) => t.id === tileId) ?? null;

  const dirty = () => {
    timers.current.forEach((t) => window.clearTimeout(t));
    timers.current = [];
    setSend('unsent');
  };
  const editPage = (fn: (p: Page) => Page) => {
    setPages((all) => all.map((p) => (p.id === page.id ? fn(p) : p)));
    dirty();
  };

  return {
    screen,
    setScreen,
    deviceName,
    rename: (name) => setDeviceName(name.trim() || deviceName),
    pages,
    page,
    selectPage: (id) => {
      setPageId(id);
      setTileId(null);
    },
    addPage: () => {
      const id = newId('page');
      setPages((all) => [...all, { id, title: `Page ${all.length + 1}`, rows: 2, cols: 4, tiles: [] }]);
      setPageId(id);
      setTileId(null);
      dirty();
    },
    renamePage: (title) => editPage((p) => ({ ...p, title })),
    setGrid: (rows, cols) => editPage((p) => ({ ...p, rows, cols, tiles: regrid(p, rows, cols) })),
    tile,
    selectTile: setTileId,
    addTile: (cell) => {
      const used = new Set(page.tiles.map((t) => t.cell));
      let at = cell ?? 0;
      if (cell === undefined) while (used.has(at) && at < page.rows * page.cols) at++;
      if (at >= page.rows * page.cols || used.has(at)) return;
      const t: Tile = { id: newId('tile'), cell: at, label: 'New Tile', icon: 'bulb', action: ACTIONS[0] };
      editPage((p) => ({ ...p, tiles: [...p.tiles, t] }));
      setTileId(t.id);
    },
    updateTile: (patch) => editPage((p) => ({ ...p, tiles: p.tiles.map((t) => (t.id === tileId ? { ...t, ...patch } : t)) })),
    deleteTile: () => {
      editPage((p) => ({ ...p, tiles: p.tiles.filter((t) => t.id !== tileId) }));
      setTileId(null);
    },
    orientation,
    setOrientation,
    preview,
    setPreview: (on) => {
      setPreviewState(on);
      if (on) setTileId(null);
    },
    send,
    sendNow: () => {
      setSend('sending');
      timers.current.push(
        window.setTimeout(() => setSend('saved'), 1100),
        window.setTimeout(() => setSend('hidden'), 2600),
      );
    },
  };
}

/* ------------------------------------------------------------------ nav */

const DESTINATIONS: { screen: Screen; label: string; icon: ComponentType<LucideProps> }[] = [
  { screen: 'home', label: 'Home', icon: House },
  { screen: 'layout', label: 'Layout', icon: LayoutGrid },
  { screen: 'integrations', label: 'Integrations', icon: Cable },
  { screen: 'settings', label: 'Settings', icon: Settings },
];

/** The four destinations stacked vertically. Where the rail sits is the variant's call. */
export function NavRail({ deck, className }: { deck: Deck; className?: string }) {
  return (
    <nav className={clsx('proto-rail', className)} aria-label="Main">
      {DESTINATIONS.map((d) => (
        <button
          key={d.screen}
          type="button"
          className={clsx('nav-button', deck.screen === d.screen && 'is-current')}
          aria-label={d.label}
          title={d.label}
          aria-current={deck.screen === d.screen ? 'page' : undefined}
          onClick={() => deck.setScreen(d.screen)}
        >
          <d.icon size={20} strokeWidth={1.9} aria-hidden />
        </button>
      ))}
    </nav>
  );
}

/** What the other rail destinations show: nothing real, just proof that the rail doesn't move. */
export function OtherScreen({ screen }: { screen: Screen }) {
  const name = DESTINATIONS.find((d) => d.screen === screen)?.label ?? screen;
  return (
    <div className="proto-other">
      <p className="proto-other-title">{name}</p>
      <p className="proto-other-sub">Not part of this prototype. The rail stayed where it was.</p>
    </div>
  );
}

/* ------------------------------------------------------------ device name */

/** The Device's name with a pencil. The pencil turns the name into a field; Enter or leaving saves, Escape cancels. */
export function DeviceName({ deck }: { deck: Deck }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(deck.deviceName);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (editing) input.current?.select();
  }, [editing]);
  const done = (save: boolean) => {
    if (save) deck.rename(draft);
    else setDraft(deck.deviceName);
    setEditing(false);
  };
  const keys = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') done(true);
    if (event.key === 'Escape') done(false);
  };
  if (editing) {
    return (
      <input
        ref={input}
        className="input proto-name-input"
        value={draft}
        maxLength={24}
        aria-label="Device name"
        onChange={(event) => setDraft(event.target.value)}
        onBlur={() => done(true)}
        onKeyDown={keys}
      />
    );
  }
  return (
    <div className="proto-name">
      <span className="proto-name-text">{deck.deviceName}</span>
      <button
        type="button"
        className="button button-icon proto-name-pencil"
        aria-label={`Rename ${deck.deviceName}`}
        title={`Rename ${deck.deviceName}`}
        onClick={() => {
          setDraft(deck.deviceName);
          setEditing(true);
        }}
      >
        <Pencil size={16} strokeWidth={1.9} aria-hidden />
      </button>
    </div>
  );
}

/* ---------------------------------------------------------------- device */

export function DeviceScreen({ deck, onTile, onEmpty }: { deck: Deck; onTile?(tile: Tile): void; onEmpty?(cell: number): void }) {
  const { page, orientation, preview, tile: selected } = deck;
  const cells = Array.from({ length: page.rows * page.cols }, (_, i) => i);
  const byCell = new Map(page.tiles.map((t) => [t.cell, t]));
  return (
    <div className={clsx('proto-device', `is-${orientation}`)} data-preview={preview || undefined}>
      <div className="proto-screen" onClick={() => !preview && deck.selectTile(null)}>
        <span className="proto-screen-title">{page.title}</span>
        <div className="proto-grid" style={{ gridTemplateColumns: `repeat(${page.cols}, 1fr)`, gridTemplateRows: `repeat(${page.rows}, 1fr)` }}>
          {cells.map((cell) => {
            const t = byCell.get(cell);
            if (!t) {
              return preview ? (
                <span key={cell} />
              ) : (
                <button
                  key={cell}
                  type="button"
                  className="proto-empty"
                  aria-label="Add a Tile here"
                  onClick={(event) => {
                    event.stopPropagation();
                    onEmpty?.(cell);
                  }}
                >
                  <Plus size={16} strokeWidth={2} aria-hidden />
                </button>
              );
            }
            return (
              <button
                key={t.id}
                type="button"
                className={clsx('proto-tile', selected?.id === t.id && 'is-selected', t.value && 'has-value')}
                data-tile-id={t.id}
                onClick={(event) => {
                  event.stopPropagation();
                  if (!preview) onTile?.(t);
                }}
              >
                <Icon k={t.icon} size={t.value ? 16 : 22} />
                {t.value && <span className="proto-tile-value">{t.value}</span>}
                <span className="proto-tile-label">{t.label}</span>
              </button>
            );
          })}
        </div>
        <span className="proto-pager" aria-hidden>
          {deck.pages.map((p) => (
            <span key={p.id} className={clsx('proto-pager-dot', p.id === page.id && 'is-current')} />
          ))}
        </span>
      </div>
    </div>
  );
}

/* ----------------------------------------------------------------- pages */

export function PageThumb({ page }: { page: Page }) {
  const used = new Set(page.tiles.map((t) => t.cell));
  return (
    <span className="proto-thumb" style={{ gridTemplateColumns: `repeat(${page.cols}, 1fr)`, gridTemplateRows: `repeat(${page.rows}, 1fr)` }} aria-hidden>
      {Array.from({ length: page.rows * page.cols }, (_, i) => (
        <span key={i} className={clsx('proto-thumb-cell', used.has(i) && 'is-used')} />
      ))}
    </span>
  );
}

/** The MX-style list: a row per Page with its thumbnail, name and Tile count. */
export function PageList({ deck }: { deck: Deck }) {
  return (
    <div className="proto-pages">
      <p className="proto-pages-head">Pages</p>
      <ul className="proto-pages-list">
        {deck.pages.map((p) => (
          <li key={p.id}>
            <button
              type="button"
              className={clsx('proto-page-row', p.id === deck.page.id && 'is-current')}
              aria-current={p.id === deck.page.id ? 'true' : undefined}
              onClick={() => deck.selectPage(p.id)}
            >
              <PageThumb page={p} />
              <span className="proto-page-text">
                <span className="proto-page-name">{p.title}</span>
                <span className="proto-page-sub">
                  {p.tiles.length} {p.tiles.length === 1 ? 'Tile' : 'Tiles'}
                </span>
              </span>
            </button>
          </li>
        ))}
      </ul>
      <button type="button" className="button proto-add-page" onClick={deck.addPage}>
        <Plus size={16} strokeWidth={2} aria-hidden />
        Add Page
      </button>
    </div>
  );
}

/* ---------------------------------------------------------------- fields */

/** Send to Device: shows once there's something to send, spins while sending, says Saved, then goes. */
export function SendButton({ deck, className }: { deck: Deck; className?: string }) {
  if (deck.send === 'hidden') return null;
  const busy = deck.send === 'sending';
  const saved = deck.send === 'saved';
  return (
    <button
      type="button"
      className={clsx('button button-primary proto-send', saved && 'is-saved', className)}
      aria-disabled={busy || saved}
      onClick={() => deck.send === 'unsent' && deck.sendNow()}
    >
      {busy ? <LoaderCircle className="proto-spin" size={16} strokeWidth={2} aria-hidden /> : saved ? <Check size={16} strokeWidth={2.2} aria-hidden /> : <Upload size={16} strokeWidth={2} aria-hidden />}
      <span>{busy ? 'Sending…' : saved ? 'Saved' : 'Send to Device'}</span>
    </button>
  );
}

export function OrientationToggle({ deck }: { deck: Deck }) {
  return (
    <div className="segmented segmented-icons" role="group" aria-label="Orientation">
      {(['landscape', 'portrait'] as const).map((o) => (
        <button
          key={o}
          type="button"
          className="segment"
          data-pressed={deck.orientation === o ? '' : undefined}
          aria-pressed={deck.orientation === o}
          aria-label={o === 'landscape' ? 'Landscape' : 'Portrait'}
          title={o === 'landscape' ? 'Landscape' : 'Portrait'}
          onClick={() => deck.setOrientation(o)}
        >
          <span className={clsx('proto-orient', `is-${o}`)} aria-hidden />
        </button>
      ))}
    </div>
  );
}

const MAX = 8;

/** The 8 × 8 board: sweep to preview, click to set. */
export function GridPicker({ rows, cols, onPick }: { rows: number; cols: number; onPick(rows: number, cols: number): void }) {
  const [hover, setHover] = useState<{ r: number; c: number } | null>(null);
  const shown = hover ?? { r: rows, c: cols };
  return (
    <div className="proto-gridpick">
      <span className="proto-gridpick-size">
        {shown.r} rows × {shown.c} columns
      </span>
      <div className="proto-gridpick-board" onPointerLeave={() => setHover(null)}>
        {Array.from({ length: MAX * MAX }, (_, i) => {
          const r = Math.floor(i / MAX) + 1;
          const c = (i % MAX) + 1;
          const lit = hover ? r <= hover.r && c <= hover.c && 'preview' : r <= rows && c <= cols && 'set';
          return (
            <button
              key={i}
              type="button"
              className="proto-gridpick-cell"
              data-lit={lit || undefined}
              aria-label={`${r} rows × ${c} columns`}
              onPointerEnter={() => setHover({ r, c })}
              onClick={() => onPick(r, c)}
            />
          );
        })}
      </div>
    </div>
  );
}

const ICON_CHOICES = Object.keys(ICONS) as IconKey[];

export function IconChoice({ value, onPick, up }: { value: IconKey; onPick(icon: IconKey): void; up?: boolean }) {
  const [open, setOpen] = useState(false);
  return (
    <span className="proto-iconchoice">
      <button type="button" className="button button-icon proto-iconchoice-button" aria-label="Icon" title="Icon" aria-expanded={open} onClick={() => setOpen(!open)}>
        <Icon k={value} />
      </button>
      {open && (
        <span className={clsx('proto-iconchoice-pop', up && 'is-up')}>
          {ICON_CHOICES.map((k) => (
            <button
              key={k}
              type="button"
              className={clsx('button button-icon', k === value && 'is-current')}
              aria-label={k}
              onClick={() => {
                onPick(k);
                setOpen(false);
              }}
            >
              <Icon k={k} size={16} />
            </button>
          ))}
        </span>
      )}
    </span>
  );
}

export function ActionSelect({ value, onChange, className }: { value: string; onChange(action: string): void; className?: string }) {
  return (
    <select className={clsx('input proto-select', className)} value={value} aria-label="Action" onChange={(event) => onChange(event.target.value)}>
      {ACTIONS.map((a) => (
        <option key={a} value={a}>
          {a}
        </option>
      ))}
    </select>
  );
}
