import { useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type RefObject } from 'react';
import clsx from 'clsx';
import { AppWindow, PackagePlus, Search, Upload } from 'lucide-react';
import type { AssetRef, Builtin, CatalogHit } from '../../lib/companion/api.ts';
import type { Companion } from '../../lib/companion/client.ts';
import { useCompanion } from '../../lib/companion/connection.ts';
import { toApiError } from '../../lib/companion/errors.ts';
import { jobsStore, waitForJob } from '../../lib/companion/jobs.ts';
import type { EditorStore } from '../../lib/editor/store.ts';
import { assetOf, type Tile } from '../../lib/layout/model.ts';
import { catalogTint, createCatalogSearch, monogram, SOURCE_NAMES, type CatalogSearch, type SearchState } from '../../lib/icons/catalog.ts';
import { moveSpot, type Spot } from '../../lib/icons/grid.ts';
import { appProblem, catalogProblem, ICONS_FAILED, renderApp, renderCatalog, renderUpload, searchProblem, uploadProblem, type PickDeps } from '../../lib/icons/pick.ts';
import { forget, readRecent, recentKey, remember, writeRecent, type RecentIcon } from '../../lib/icons/recent.ts';
import { useIconPacks } from '../../lib/icon-packs/hooks.ts';
import { credits, packPrefixOf, sourceName } from '../../lib/icon-packs/model.ts';
import { useUi } from '../../state/ui.ts';
import { button } from '../../shell/button.ts';
import { AssetImage } from './AssetImage.tsx';
import { BuiltinIcon, SYMBOL_NAMES } from './glyphs.tsx';
import { PackThumbnail } from './PackThumbnail.tsx';

const COLS = 4;

type Item =
  | { kind: 'none'; key: string; name: string }
  | { kind: 'symbol'; key: string; icon: string; name: string }
  | { kind: 'file'; key: string; id: string; name: string }
  | { kind: 'asset'; key: string; asset: string; name: string }
  | { kind: 'hit'; key: string; hit: CatalogHit; name: string };

interface Section {
  id: 'recent' | 'logos' | 'builtin';
  title: string;
  items: Item[];
}

type Work = { kind: 'idle' } | { kind: 'busy'; text: string } | { kind: 'failed'; text: string; detail: string; retry: () => void };

const NONE: Item = { kind: 'none', key: 'none', name: 'No icon' };
const SYMBOLS: Item[] = Object.entries(SYMBOL_NAMES).map(([icon, name]) => ({ kind: 'symbol', key: icon, icon, name }));

function storage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

const builtinLists = new WeakMap<Companion, Builtin[]>();

function recentItem(icon: RecentIcon): Item {
  const key = recentKey(icon);
  switch (icon.kind) {
    case 'asset':
      return { kind: 'asset', key, asset: icon.asset, name: icon.name };
    case 'file':
      return { kind: 'file', key, id: icon.id, name: icon.name };
    case 'symbol':
      return { kind: 'symbol', key, icon: icon.icon, name: icon.name };
    default: {
      const exhaustive: never = icon;
      return exhaustive;
    }
  }
}

const builtinItem = (icon: Builtin): Item => ({ kind: 'file', key: `file:${icon.id}`, id: icon.id, name: icon.label });

function matches(words: string[], ...texts: string[]) {
  const text = texts.join(' ').toLowerCase();
  return words.every((word) => text.includes(word));
}

function currentKey(tile: Pick<Tile, 'icon' | 'asset'>): string {
  const asset = assetOf(tile);
  if (asset) return `asset:${asset}`;
  return tile.icon ?? 'none';
}

const hex = (rgb: readonly number[]) => `#${rgb.map((c) => c.toString(16).padStart(2, '0')).join('')}`;

function Glyph({ item, onMissing }: { item: Item; onMissing(): void }) {
  switch (item.kind) {
    case 'none':
      return <span className="icon-cell-none" aria-hidden />;
    case 'symbol':
      return <BuiltinIcon icon={item.icon} size={22} />;
    case 'file':
      return <AssetImage id={item.id} size={32} onMissing={onMissing} />;
    case 'asset':
      return <AssetImage id={item.asset} size={32} onMissing={onMissing} />;
    case 'hit': {
      const pack = packPrefixOf(item.hit.source);
      if (pack !== null) return <PackThumbnail prefix={pack} name={item.hit.id} size={32} />;
      const tint = catalogTint(item.hit);
      return (
        <span className="icon-cell-mark" style={tint ? ({ color: hex(tint) } satisfies CSSProperties) : undefined} aria-hidden>
          {monogram(item.name)}
        </span>
      );
    }
    default: {
      const exhaustive: never = item;
      return exhaustive;
    }
  }
}

export interface IconPickerProps {
  store: EditorStore;
  tile: Tile;
  searchRef: RefObject<HTMLInputElement | null>;
  onDone(): void;
}

export function IconPicker({ store, tile, searchRef, onDone }: IconPickerProps) {
  const companion = useCompanion();
  const [query, setQuery] = useState('');
  const [search, setSearch] = useState<SearchState>({ kind: 'idle' });
  const [recent, setRecent] = useState(() => readRecent(storage()));
  const [builtins, setBuiltins] = useState<Builtin[] | 'failed' | null>(() => builtinLists.get(companion) ?? null);
  const [work, setWork] = useState<Work>({ kind: 'idle' });
  const [spot, setSpot] = useState<Spot>({ section: 0, index: 0 });
  const searcher = useRef<CatalogSearch | null>(null);
  const running = useRef<AbortController | null>(null);
  const cells = useRef(new Map<string, HTMLButtonElement>());
  const file = useRef<HTMLInputElement>(null);
  const packList = useIconPacks(companion, (s) => s.packs);
  const openIconPacks = useUi((s) => s.openIconPacks);
  const deps = useMemo<PickDeps>(() => ({ companion, wait: (job, signal) => waitForJob<'assets.render'>(jobsStore, companion, job, { signal }) }), [companion]);

  useEffect(() => {
    const controller = createCatalogSearch({
      search: (q, signal) => companion.assets.catalog_search({}, { q, catalog: null }, null, undefined, undefined, signal),
      onChange: setSearch,
    });
    searcher.current = controller;
    return () => {
      controller.close();
      running.current?.abort();
    };
  }, [companion]);

  useEffect(() => {
    if (builtinLists.has(companion)) return;
    const controller = new AbortController();
    companion.assets.builtin({}, null, null, undefined, undefined, controller.signal).then(
      (list) => {
        builtinLists.set(companion, list);
        setBuiltins(list);
      },
      () => {
        if (!controller.signal.aborted) setBuiltins('failed');
      },
    );
    return () => controller.abort();
  }, [companion]);

  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const list = Array.isArray(builtins) ? builtins : [];
  const hits = search.kind === 'done' || search.kind === 'loading' ? search.hits : [];
  const offered: Section[] = words.length
      ? [
          { id: 'logos', title: 'Logos', items: hits.map((hit): Item => ({ kind: 'hit', key: `hit:${hit.source}:${hit.id}`, hit, name: hit.name })) },
          {
            id: 'builtin',
            title: 'Built-in',
            items: [...SYMBOLS.filter((item) => matches(words, item.name)), ...list.filter((icon) => matches(words, icon.name, icon.label, ...icon.tags)).map(builtinItem)],
          },
        ]
      : [
          { id: 'recent', title: 'Recent', items: recent.map(recentItem) },
          { id: 'builtin', title: 'Built-in', items: [NONE, ...SYMBOLS, ...list.map(builtinItem)] },
        ];
  const sections = offered.filter((section) => section.items.length > 0);

  const sizes = sections.map((section) => section.items.length);
  const active = sections[spot.section]?.items[spot.index] ? spot : { section: 0, index: 0 };
  const current = currentKey(tile);

  const keep = (icon: RecentIcon) => {
    const next = remember(recent, icon);
    setRecent(next);
    writeRecent(storage(), next);
  };

  const drop = (key: string) =>
    setRecent((now) => {
      const next = forget(now, key);
      writeRecent(storage(), next);
      return next;
    });

  const run = (text: string, task: (signal: AbortSignal) => Promise<AssetRef>, use: (ref: AssetRef) => void, problem: (error: unknown) => string) => {
    running.current?.abort();
    const controller = new AbortController();
    running.current = controller;
    setWork({ kind: 'busy', text });
    task(controller.signal).then(
      (ref) => {
        if (controller.signal.aborted) return;
        use(ref);
        onDone();
      },
      (error: unknown) => {
        if (controller.signal.aborted) return;
        setWork({ kind: 'failed', text: problem(error), detail: toApiError(error).message, retry: () => run(text, task, use, problem) });
      },
    );
  };

  const pick = (item: Item) => {
    switch (item.kind) {
      case 'none':
        store.setIcon(tile.id, null);
        return onDone();
      case 'symbol':
        store.setIcon(tile.id, item.icon);
        keep({ kind: 'symbol', icon: item.icon, name: item.name });
        return onDone();
      case 'file':
        store.setIcon(tile.id, `file:${item.id}`);
        keep({ kind: 'file', id: item.id, name: item.name });
        return onDone();
      case 'asset':
        store.setAsset(tile.id, item.asset);
        keep({ kind: 'asset', asset: item.asset, name: item.name });
        return onDone();
      case 'hit':
        return run(
          `Getting ${item.name}…`,
          (signal) => renderCatalog(deps, item.hit, signal),
          (ref) => {
            store.setAsset(tile.id, ref.id);
            keep({ kind: 'asset', asset: ref.id, name: item.name });
          },
          (error) => catalogProblem(error, item.name),
        );
      default: {
        const exhaustive: never = item;
        return exhaustive;
      }
    }
  };

  const upload = (chosen: File) =>
    run(
      `Uploading “${chosen.name}”…`,
      (signal) => renderUpload(deps, chosen, signal),
      (ref) => {
        store.setAsset(tile.id, ref.id);
        keep({ kind: 'asset', asset: ref.id, name: chosen.name });
      },
      (error) => uploadProblem(error, chosen.name),
    );

  const appPath = tile.action.type === 'launch' ? tile.action.path : '';
  const takeAppIcon = () =>
    run(
      'Getting icon…',
      (signal) => renderApp(deps, appPath, signal),
      (ref) => {
        store.setAsset(tile.id, ref.id);
        keep({ kind: 'asset', asset: ref.id, name: appPath.split(/[\\/]/).pop() || 'App icon' });
      },
      appProblem,
    );

  const focusCell = (at: Spot) => {
    setSpot(at);
    cells.current.get(`${at.section}:${at.index}`)?.focus();
  };

  const gridKeys = (event: KeyboardEvent<HTMLDivElement>) => {
    const next = moveSpot(sizes, active, event.key, COLS);
    if (next === null) return;
    event.preventDefault();
    if (next === 'search') searchRef.current?.focus();
    else focusCell(next);
  };

  const searchKeys = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown' && sizes.length) {
      event.preventDefault();
      focusCell(active);
    }
    if (event.key === 'Enter' && words.length && sections[0]) {
      event.preventDefault();
      pick(sections[0].items[0]);
    }
  };

  const typeAhead = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.target === searchRef.current || event.key.length !== 1 || event.key === ' ' || event.ctrlKey || event.metaKey || event.altKey) return;
    searchRef.current?.focus();
  };

  const searching = search.kind === 'loading' && hits.length === 0;
  const nothing = words.length > 0 && search.kind === 'done' && sections.length === 0;
  const busy = work.kind === 'busy';

  return (
    <div className="icon-picker-body" onKeyDown={typeAhead}>
      <div className="icon-search">
        <Search className="icon-search-glyph" size={16} strokeWidth={1.9} aria-hidden />
        <input
          ref={searchRef}
          type="search"
          className="input icon-search-input"
          placeholder="Search icons, like Spotify or mail"
          aria-label="Search icons"
          value={query}
          spellCheck={false}
          autoComplete="off"
          onChange={(event) => {
            setQuery(event.target.value);
            setSpot({ section: 0, index: 0 });
            searcher.current?.type(event.target.value);
          }}
          onKeyDown={searchKeys}
          data-testid="icon-search"
        />
      </div>
      {!words.length && recent.length === 0 && <p className="field-hint icon-picker-hint">Search Simple Icons and selfh.st logos, or pick a built-in icon.</p>}
      <div className="icon-grid" onKeyDown={gridKeys} aria-busy={search.kind === 'loading'} data-testid="icon-grid" data-state={search.kind}>
        {sections.map((section, s) => (
          <div className="icon-section" role="group" aria-label={section.title} key={section.id} data-testid={`icon-section-${section.id}`}>
            <h3 className="icon-section-title" aria-hidden>
              {section.title}
            </h3>
            <div className="icon-cells">
              {section.items.map((item, i) => {
                const here = active.section === s && active.index === i;
                const source = item.kind === 'hit' ? sourceName(item.hit, SOURCE_NAMES) : null;
                return (
                  <button
                    key={item.key}
                    ref={(el) => {
                      if (el) cells.current.set(`${s}:${i}`, el);
                      else cells.current.delete(`${s}:${i}`);
                    }}
                    type="button"
                    className={clsx('icon-cell', item.key === current && 'is-current')}
                    tabIndex={here ? 0 : -1}
                    aria-disabled={busy || undefined}
                    aria-label={source ? `${item.name}, ${source}` : item.name}
                    aria-current={item.key === current ? 'true' : undefined}
                    title={source ? `${item.name} · ${source}` : item.name}
                    onFocus={() => setSpot({ section: s, index: i })}
                    onClick={() => {
                      if (!busy) pick(item);
                    }}
                    data-testid="icon-option"
                    data-kind={item.kind}
                    data-key={item.key}
                  >
                    <span className="icon-cell-tile">
                      <Glyph item={item} onMissing={() => drop(item.key)} />
                    </span>
                    <span className="icon-cell-name">{item.name}</span>
                    {source && <span className="icon-cell-source">{source}</span>}
                  </button>
                );
              })}
            </div>
          </div>
        ))}
        <div className="icon-grid-status" role="status" data-testid="icon-status">
          {searching && <p className="field-hint">Searching…</p>}
          {nothing && <p className="field-hint">No icons match “{query.trim()}”. Try a shorter word.</p>}
          {search.kind === 'failed' && (
            <div className="icon-problem" data-testid="icon-search-error">
              <p className="field-error">{searchProblem(search.error)}</p>
              <button type="button" className={button({ size: 'small' })} onClick={() => searcher.current?.retry()}>
                Retry
              </button>
            </div>
          )}
          {builtins === 'failed' && !words.length && <p className="field-error">{ICONS_FAILED}</p>}
        </div>
      </div>
      <div className="icon-work" role="status" data-testid="icon-work" data-state={work.kind}>
        {work.kind === 'busy' && <p className="field-hint">{work.text}</p>}
        {work.kind === 'failed' && (
          <div className="icon-problem" data-testid="icon-error">
            <p className="field-error">{work.text}</p>
            <div className="icon-problem-actions">
              <button type="button" className={button({ size: 'small' })} onClick={work.retry} data-testid="icon-retry">
                Retry
              </button>
              <details className="icon-details">
                <summary>Details</summary>
                <p>{work.detail}</p>
              </details>
            </div>
          </div>
        )}
      </div>
      <div className="icon-picker-foot">
        <button type="button" className={button({ size: 'small' })} aria-disabled={busy || undefined} onClick={() => !busy && file.current?.click()} data-testid="icon-upload">
          <Upload size={14} strokeWidth={1.9} aria-hidden />
          Upload image
        </button>
        {appPath && (
          <button type="button" className={button({ size: 'small' })} aria-disabled={busy || undefined} onClick={() => !busy && takeAppIcon()} data-testid="icon-app">
            <AppWindow size={14} strokeWidth={1.9} aria-hidden />
            Use app icon
          </button>
        )}
        <button
          type="button"
          className={button({ size: 'small' })}
          onClick={() => {
            onDone();
            openIconPacks();
          }}
          data-testid="icon-more"
        >
          <PackagePlus size={14} strokeWidth={1.9} aria-hidden />
          Get more icons
        </button>
        <input
          ref={file}
          type="file"
          accept="image/png,image/jpeg,image/svg+xml"
          hidden
          onChange={(event) => {
            const chosen = event.target.files?.[0];
            event.target.value = '';
            if (chosen) upload(chosen);
          }}
          data-testid="icon-upload-file"
        />
      </div>
      <p className="icon-credits">
        Icons:{' '}
        <a href="https://simpleicons.org" target="_blank" rel="noreferrer">
          Simple Icons
        </a>{' '}
        (CC0) ·{' '}
        <a href="https://selfh.st/icons" target="_blank" rel="noreferrer">
          selfh.st/icons
        </a>{' '}
        (CC BY 4.0, modified)
        {credits(packList ?? []).map((credit) => (
          <span key={credit.prefix} data-testid="icon-credit" data-prefix={credit.prefix}>
            {' '}
            ·{' '}
            {credit.url ? (
              <a href={credit.url} target="_blank" rel="noreferrer">
                {credit.name}
              </a>
            ) : (
              credit.name
            )}{' '}
            by {credit.author} ({credit.license}, modified)
          </span>
        ))}{' '}
        · Logos belong to their owners
      </p>
    </div>
  );
}
