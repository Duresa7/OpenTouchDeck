// PROTOTYPE — throwaway, answers: how should the Layout editor lay out so the nav never shifts?
// Three variants of the Layout editor on /prototype.html, switchable with ?variant=a|b|c, the floating switcher at the
// bottom left, or the ← and → keys. Run `pnpm dev:prototype` from app/ui and open http://127.0.0.1:5181/prototype.html
import '@fontsource-variable/archivo/wdth.css';
import '../styles/tokens.css';
import '../styles/app.css';
import './layout-editor.prototype.css';
import { StrictMode, useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ChevronLeft, ChevronRight, SunMoon } from 'lucide-react';
import { useDeck, type Deck } from './layout-editor.prototype-parts.tsx';
import { VariantA, VariantB, VariantC } from './layout-editor.prototype-variants.tsx';

const VARIANTS = [
  { key: 'a', name: 'Rail + Page list + overlay Details', View: VariantA },
  { key: 'b', name: 'Rail + Page list + toolbar', View: VariantB },
  { key: 'c', name: 'Rail + bottom contextual toolbar', View: VariantC },
] as const;

type Theme = 'system' | 'light' | 'dark';
const THEMES: Theme[] = ['system', 'light', 'dark'];

const readVariant = () => {
  const v = new URLSearchParams(window.location.search).get('variant')?.toLowerCase();
  const i = VARIANTS.findIndex((x) => x.key === v);
  return i < 0 ? 0 : i;
};

function typing(target: EventTarget | null) {
  return target instanceof HTMLElement && (target.matches('input, textarea, select') || target.isContentEditable);
}

function stateLine(deck: Deck) {
  if (deck.screen !== 'layout') return `Screen: ${deck.screen}`;
  return [
    `Page: ${deck.page.title} (${deck.page.rows}×${deck.page.cols})`,
    `Tile: ${deck.tile ? deck.tile.label : 'none'}`,
    deck.orientation,
    deck.preview ? 'preview' : 'edit',
    `Send: ${deck.send}`,
  ].join(' · ');
}

function Switcher({ index, onGo, deck, theme, onTheme }: { index: number; onGo(i: number): void; deck: Deck; theme: Theme; onTheme(): void }) {
  const v = VARIANTS[index]!;
  const step = (by: number) => onGo((index + by + VARIANTS.length) % VARIANTS.length);
  return (
    <div className="proto-switcher" role="toolbar" aria-label="Prototype variants">
      <div className="proto-switcher-row">
        <button type="button" aria-label="Previous variant" onClick={() => step(-1)}>
          <ChevronLeft size={16} strokeWidth={2.2} aria-hidden />
        </button>
        <span className="proto-switcher-label">
          {v.key.toUpperCase()} ({v.name})
        </span>
        <button type="button" aria-label="Next variant" onClick={() => step(1)}>
          <ChevronRight size={16} strokeWidth={2.2} aria-hidden />
        </button>
        <button type="button" className="proto-switcher-theme" aria-label={`Theme: ${theme}`} title={`Theme: ${theme}`} onClick={onTheme}>
          <SunMoon size={15} strokeWidth={2} aria-hidden />
          <span>{theme}</span>
        </button>
      </div>
      <span className="proto-switcher-state">{stateLine(deck)}</span>
    </div>
  );
}

function Prototype() {
  const deck = useDeck();
  const [index, setIndex] = useState(readVariant);
  const [theme, setTheme] = useState<Theme>('system');
  const go = (i: number) => {
    setIndex(i);
    const url = new URL(window.location.href);
    url.searchParams.set('variant', VARIANTS[i]!.key);
    window.history.replaceState(null, '', url);
  };

  useEffect(() => {
    if (theme === 'system') document.documentElement.removeAttribute('data-theme');
    else document.documentElement.setAttribute('data-theme', theme);
  }, [theme]);

  useEffect(() => {
    const keys = (event: KeyboardEvent) => {
      if (typing(event.target) || event.altKey || event.ctrlKey || event.metaKey) return;
      if (event.key === 'ArrowLeft') go((index + VARIANTS.length - 1) % VARIANTS.length);
      if (event.key === 'ArrowRight') go((index + 1) % VARIANTS.length);
      if (event.key === 'p' || event.key === 'P') deck.setPreview(!deck.preview);
      if (event.key === 'Escape') deck.selectTile(null);
    };
    window.addEventListener('keydown', keys);
    return () => window.removeEventListener('keydown', keys);
  });

  const { View } = VARIANTS[index]!;
  return (
    <>
      <View deck={deck} />
      {import.meta.env.DEV && (
        <Switcher index={index} onGo={go} deck={deck} theme={theme} onTheme={() => setTheme(THEMES[(THEMES.indexOf(theme) + 1) % THEMES.length]!)} />
      )}
    </>
  );
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <Prototype />
  </StrictMode>,
);
