import { useEffect, useLayoutEffect, useRef } from 'react';
import { Tooltip } from '@base-ui/react/tooltip';
import type { IntegrationSummary } from '../lib/companion/api.ts';
import { useIntegrations } from '../lib/companion/integrations.ts';
import { BRANDFETCH, ICON_PACKS, useUi } from '../state/ui.ts';
import { BRANDFETCH_LEDE, BrandfetchBody } from './integrations/Brandfetch.tsx';
import { ICON_PACKS_LEDE, IconPacksBody } from './integrations/IconPacks.tsx';
import { IntegrationDetail } from './integrations/IntegrationDetail.tsx';
import { IntegrationList } from './integrations/IntegrationList.tsx';
import { SettingsPage } from './integrations/SettingsPage.tsx';
import { useBrandfetchSwitch, useIconPacksSwitch } from './integrations/switches.ts';
import '../styles/integrations.css';

function useTitleFocus() {
  const title = useRef<HTMLHeadingElement>(null);
  useEffect(() => title.current?.focus({ preventScroll: true }), []);
  return title;
}

function IconPacksPage({ onBack }: { onBack(): void }) {
  const state = useIconPacksSwitch();
  const title = useTitleFocus();
  return (
    <SettingsPage id={ICON_PACKS} name="Icon packs" icon="shapes" lede={ICON_PACKS_LEDE} state={state} titleRef={title} onBack={onBack}>
      <IconPacksBody />
    </SettingsPage>
  );
}

function BrandfetchPage({ onBack }: { onBack(): void }) {
  const state = useBrandfetchSwitch();
  const title = useTitleFocus();
  return (
    <SettingsPage id={BRANDFETCH} name="Brandfetch logos" icon="logos" lede={BRANDFETCH_LEDE} state={state} titleRef={title} onBack={onBack}>
      <BrandfetchBody />
    </SettingsPage>
  );
}

function Page({ id, setUp, integrations, onBack }: { id: string; setUp: boolean; integrations: readonly IntegrationSummary[]; onBack(): void }) {
  if (id === ICON_PACKS) return <IconPacksPage onBack={onBack} />;
  if (id === BRANDFETCH) return <BrandfetchPage onBack={onBack} />;
  const summary = integrations.find((i) => i.id === id);
  return summary ? <IntegrationDetail key={summary.id} summary={summary} setUp={setUp} onBack={onBack} /> : null;
}

/**
 * Integrations: turn each on or off, and open the settings of those that have some. Their Tiles are added from
 * Layout. The settings page shown lives in the UI store, so other screens can open one.
 */
export function Integrations() {
  const integrations = useIntegrations((s) => s.integrations);
  const open = useUi((s) => s.integration);
  const openIntegration = useUi((s) => s.openIntegration);
  const closeIntegration = useUi((s) => s.closeIntegration);
  const back = useRef<string | null>(null);
  const list = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    if (open || !back.current) return;
    list.current?.querySelector<HTMLElement>(`[data-id="${CSS.escape(back.current)}"] [data-testid="integration-settings"]`)?.focus();
    back.current = null;
  }, [open]);

  return (
    <Tooltip.Provider delay={500} closeDelay={0}>
      <main className="integrations" data-testid="integrations">
        {open ? (
          <Page
            id={open.id}
            setUp={open.setUp}
            integrations={integrations}
            onBack={() => {
              back.current = open.id;
              closeIntegration();
            }}
          />
        ) : (
          <div ref={list}>
            <header className="page-head">
              <h1 className="page-title">Integrations</h1>
              <p className="page-lede">Turn on the apps and services your Tiles use. Add their Tiles from Layout.</p>
            </header>
            <IntegrationList integrations={integrations} onOpen={(id) => openIntegration(id)} />
          </div>
        )}
      </main>
    </Tooltip.Provider>
  );
}
