import type { ReactNode } from 'react';
import type { IntegrationSummary } from '../../lib/companion/api.ts';
import { lineOf, statusOf } from '../../lib/integrations/model.ts';
import { hasSettings } from '../../lib/integrations/switch.ts';
import { StatusBox } from '../../shell/StatusBox.tsx';
import { BRANDFETCH, ICON_PACKS } from '../../state/ui.ts';
import { sourceOn, useIconSources } from '../../state/icon-sources.ts';
import { Tip } from '../layout/Tip.tsx';
import { IntegrationRow } from './IntegrationRow.tsx';
import { brandfetchReady, useBrandfetchSwitch, useBrandfetchView, useIconPacksSwitch, useInstalledPacks, useIntegrationSwitch } from './switches.ts';

/** The line under a Companion Integration's name: what it does, or, while it's on, what's wrong with it. */
function Line({ summary, onSettings }: { summary: IntegrationSummary; onSettings?: () => void }) {
  const status = statusOf(summary);
  const about = lineOf(summary.id);
  if (!summary.enabled || status.tone === null || status.tone === 'ok') return about;
  const box = <StatusBox tone={status.tone} label={status.words} showWords testId="integration-status" />;
  if (status.tone !== 'bad') return box;
  if (onSettings) {
    return (
      <span className="integration-problem">
        {box}
        <span aria-hidden>·</span>
        <button type="button" className="link-button" onClick={onSettings} data-testid="integration-details">
          Details
        </button>
      </span>
    );
  }
  return (
    <Tip label={status.detail ?? status.words}>
      <span className="integration-problem" tabIndex={0} data-testid="integration-problem">
        {box}
      </span>
    </Tip>
  );
}

function CompanionRow({ summary, onOpen }: { summary: IntegrationSummary; onOpen(id: string): void }) {
  const state = useIntegrationSwitch(summary);
  const onSettings = hasSettings(summary) ? () => onOpen(summary.id) : undefined;
  return (
    <IntegrationRow
      id={summary.id}
      name={summary.name}
      icon={summary.icon}
      line={<Line summary={summary} onSettings={onSettings} />}
      onSettings={onSettings}
      state={state}
      data={{ 'data-state': summary.status.state, 'data-delivery': summary.status.delivery.length }}
    />
  );
}

function names(list: readonly { name: string }[]) {
  return list.map((p) => p.name).join(', ');
}

function IconPacksRow({ onOpen }: { onOpen(id: string): void }) {
  const state = useIconPacksSwitch();
  const installed = useInstalledPacks();
  const line: ReactNode =
    installed && installed.length > 0 ? (
      <span className="integration-clamp" title={names(installed)}>
        {installed.length} installed: {names(installed)}
      </span>
    ) : installed ? (
      'None installed. Icons for the picker, searchable offline.'
    ) : (
      'Icons for the picker, searchable offline.'
    );
  return <IntegrationRow id={ICON_PACKS} name="Icon packs" icon="shapes" line={line} onSettings={() => onOpen(ICON_PACKS)} state={state} />;
}

function BrandfetchRow({ onOpen }: { onOpen(id: string): void }) {
  const state = useBrandfetchSwitch();
  const ready = brandfetchReady(useBrandfetchView());
  const stored = useIconSources((s) => s.brandfetch);
  const on = sourceOn(stored, ready === true);
  const line = on && ready === false ? <StatusBox tone="wait" label="Finish setup" showWords testId="integration-status" /> : 'Brand logos in the icon picker, with your own Brandfetch keys.';
  return <IntegrationRow id={BRANDFETCH} name="Brandfetch logos" icon="logos" line={line} onSettings={() => onOpen(BRANDFETCH)} state={state} />;
}

/** Every Integration with its switch, then the icon sources the picker searches. */
export function IntegrationList({ integrations, onOpen }: { integrations: readonly IntegrationSummary[]; onOpen(id: string): void }) {
  return (
    <>
      <ul className="integration-list" aria-label="Integrations" data-testid="integration-list">
        {integrations.map((summary) => (
          <CompanionRow key={summary.id} summary={summary} onOpen={onOpen} />
        ))}
      </ul>
      <section className="integration-group" aria-labelledby="integrations-icons">
        <h2 className="integration-group-title" id="integrations-icons">
          Icons
        </h2>
        <ul className="integration-list" aria-labelledby="integrations-icons" data-testid="icon-sources">
          <IconPacksRow onOpen={onOpen} />
          <BrandfetchRow onOpen={onOpen} />
        </ul>
      </section>
    </>
  );
}
