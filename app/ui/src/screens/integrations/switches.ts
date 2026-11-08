import { useEffect } from 'react';
import { toast } from 'sonner';
import { useStore } from 'zustand';
import type { IntegrationSummary, PackInfo } from '../../lib/companion/api.ts';
import { brandfetchStore, type BrandfetchView } from '../../lib/brandfetch/settings.ts';
import { useCompanion } from '../../lib/companion/connection.ts';
import { useIconPacks } from '../../lib/icon-packs/hooks.ts';
import { flipOf, switchView, useWrites, writesFor } from '../../lib/integrations/switch.ts';
import { iconSources, sourceOn, useIconSources } from '../../state/icon-sources.ts';
import { BRANDFETCH, ICON_PACKS, useUi } from '../../state/ui.ts';
import type { OnOff } from './IntegrationRow.tsx';

/**
 * A Companion Integration's switch. Flipping it on before the Integration is set up opens its setup and turns it on
 * once a save sets it up; off always turns it off. Either way only `enabled` is written.
 */
export function useIntegrationSwitch(summary: IntegrationSummary): OnOff {
  const companion = useCompanion();
  const writes = writesFor(companion);
  const saving = useWrites(writes, (s) => s.saving[summary.id] ?? null);
  const queued = useWrites(writes, (s) => s.writes[summary.id] ?? 0);
  const pending = useUi((s) => s.pendingEnable === summary.id);
  const openIntegration = useUi((s) => s.openIntegration);
  const setPendingEnable = useUi((s) => s.setPendingEnable);
  const view = switchView(summary.enabled, saving, pending, queued);
  return {
    ...view,
    flip(on) {
      if (view.locked) return;
      if (pending && !on) {
        setPendingEnable(null);
        if (!summary.enabled) return;
      }
      if (flipOf(summary, on) === 'set-up') {
        openIntegration(summary.id, { setUp: true, pendingEnable: true });
        return;
      }
      void writes.setEnabled(summary.id, on).then((done) => {
        if (!done) toast.error(`Couldn't turn ${on ? 'on' : 'off'} ${summary.name}.`);
      });
    },
  };
}

const installedOf = (packs: readonly PackInfo[] | null) => packs?.filter((p) => p.installed) ?? null;

/** The installed Icon packs, once the list has loaded. */
export function useInstalledPacks(): readonly PackInfo[] | null {
  const companion = useCompanion();
  const packs = useIconPacks(companion, (s) => s.packs);
  return installedOf(packs);
}

/**
 * A switch this window keeps. `ready` is whether the source works yet, null while that's unknown; once it works, its
 * setup is finished, which completes a pending enable.
 */
function useLocalSwitch(id: string, source: 'iconPacks' | 'brandfetch', ready: boolean | null): OnOff {
  const stored = useIconSources((s) => s[source]);
  const pending = useUi((s) => s.pendingEnable === id);
  const openIntegration = useUi((s) => s.openIntegration);
  const setPendingEnable = useUi((s) => s.setPendingEnable);
  // Once it works, a switch nobody has touched is on; until then it follows setup, so finishing setup turns it on.
  useEffect(() => {
    if (ready) iconSources.getState().settle(source, true);
  }, [ready, source]);
  useEffect(() => {
    if (pending && ready) {
      iconSources.getState().set(source, true);
      setPendingEnable(null);
    }
  }, [pending, ready, source, setPendingEnable]);
  const on = sourceOn(stored, ready === true);
  return {
    checked: on || pending,
    busy: pending && !on,
    locked: false,
    flip(next) {
      if (pending && !next) {
        setPendingEnable(null);
        if (on) iconSources.getState().set(source, false);
        return;
      }
      if (next && ready === false) {
        openIntegration(id, { setUp: true, pendingEnable: true });
        return;
      }
      iconSources.getState().set(source, next);
    },
  };
}

/** The Icon packs switch: on searches installed packs in the icon picker. Turning it on with none installed opens pack management. */
export function useIconPacksSwitch(): OnOff {
  const installed = useInstalledPacks();
  const ready = installed === null ? null : installed.length > 0;
  return useLocalSwitch(ICON_PACKS, 'iconPacks', ready);
}

/**
 * Brandfetch searches once it has a client ID, which also finishes its setup; null while that's unknown. Picking a
 * logo also needs the API key.
 */
export const brandfetchReady = (view: BrandfetchView): boolean | null => (view.kind === 'ready' ? view.status.client_id !== null : view.kind === 'unreadable' ? false : null);

export function useBrandfetchView(): BrandfetchView {
  const companion = useCompanion();
  const store = brandfetchStore(companion);
  useEffect(() => {
    void store.getState().load();
  }, [store]);
  return useStore(store, (s) => s.view);
}

/** The Brandfetch logos switch: on searches Brandfetch in the icon picker. Turning it on before a client ID is saved opens its page. */
export function useBrandfetchSwitch(): OnOff {
  return useLocalSwitch(BRANDFETCH, 'brandfetch', brandfetchReady(useBrandfetchView()));
}
