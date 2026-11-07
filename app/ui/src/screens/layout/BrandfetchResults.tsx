import { useCallback, useEffect, useRef, useState } from 'react';
import { useStore } from 'zustand';
import { clientIdOf, brandfetchStore } from '../../lib/brandfetch/settings.ts';
import { createBrandSearch, SEARCH_MESSAGES, type BrandHit, type BrandSearch, type BrandSearchState } from '../../lib/brandfetch/search.ts';
import type { Companion } from '../../lib/companion/client.ts';
import { monogram } from '../../lib/icons/catalog.ts';
import { button } from '../../shell/button.ts';
import './brandfetch.css';

export interface BrandSearchHook {
  /** Set only while the user turned Brandfetch on in Settings. */
  clientId: string | null;
  state: BrandSearchState;
  hits: readonly BrandHit[];
  type(query: string): void;
  retry(): void;
}

/** Live Brandfetch search for the icon picker. Hits stay in this component's state only. */
export function useBrandSearch(companion: Pick<Companion, 'brandfetch' | 'system'>): BrandSearchHook {
  const store = brandfetchStore(companion);
  const clientId = useStore(store, (s) => clientIdOf(s.view));
  const [state, setState] = useState<BrandSearchState>({ kind: 'idle' });
  const search = useRef<BrandSearch | null>(null);
  const query = useRef('');

  useEffect(() => {
    void store.getState().load();
  }, [store]);

  useEffect(() => {
    if (clientId === null) return;
    const controller = createBrandSearch({ clientId, onChange: setState });
    search.current = controller;
    controller.type(query.current);
    return () => {
      controller.close();
      search.current = null;
      setState({ kind: 'idle' });
    };
  }, [clientId]);

  const type = useCallback((next: string) => {
    query.current = next;
    search.current?.type(next);
  }, []);
  const retry = useCallback(() => search.current?.retry(), []);
  const hits = clientId !== null && (state.kind === 'done' || state.kind === 'loading') ? state.hits : [];
  return { clientId, state: clientId === null ? { kind: 'idle' } : state, hits, type, retry };
}

export const brandName = (hit: BrandHit) => hit.name ?? hit.domain;

/** The hit's logo, hotlinked from Brandfetch as its terms ask. Falls back to a letter. */
export function BrandThumb({ hit }: { hit: BrandHit }) {
  const [broken, setBroken] = useState(false);
  if (!hit.icon || broken) {
    return (
      <span className="icon-cell-mark" aria-hidden>
        {monogram(brandName(hit))}
      </span>
    );
  }
  return (
    <img
      className="brandfetch-thumb"
      src={hit.icon}
      alt=""
      width={32}
      height={32}
      // Brandfetch serves hotlinked logos only to pages that say where they are.
      referrerPolicy="origin"
      decoding="async"
      draggable={false}
      onError={() => setBroken(true)}
    />
  );
}

export function BrandSearchStatus({ search }: { search: BrandSearchHook }) {
  const { state } = search;
  if (state.kind !== 'failed') return null;
  return (
    <div className="icon-problem" data-testid="brandfetch-search-error" data-problem={state.problem}>
      <p className="field-error">{SEARCH_MESSAGES[state.problem]}</p>
      <button type="button" className={button({ size: 'small' })} onClick={search.retry}>
        Retry Brandfetch
      </button>
    </div>
  );
}
