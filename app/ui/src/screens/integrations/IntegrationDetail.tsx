import { useEffect, useId, useRef, useState } from 'react';
import clsx from 'clsx';
import { toast } from 'sonner';
import { useStore } from 'zustand';
import type { IntegrationSummary } from '../../lib/companion/api.ts';
import { useCompanion } from '../../lib/companion/connection.ts';
import { useDevices } from '../../lib/companion/devices.ts';
import { integrationsStore } from '../../lib/companion/integrations.ts';
import { createDetailStore, refreshFloorText, type DetailStore } from '../../lib/integrations/detail.ts';
import { headerLine, lineOf, outcomeAt, settingsTitle, statusOf } from '../../lib/integrations/model.ts';
import { completesPending, useWrites, writesFor } from '../../lib/integrations/switch.ts';
import { button } from '../../shell/button.ts';
import { useUi } from '../../state/ui.ts';
import { ConfigForm } from './ConfigForm.tsx';
import { SecretField } from './SecretField.tsx';
import { SettingsPage } from './SettingsPage.tsx';
import { useIntegrationSwitch } from './switches.ts';
import { UsageTable } from './UsageTable.tsx';

function useNow(everyMs: number) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), everyMs);
    return () => clearInterval(timer);
  }, [everyMs]);
  return now;
}

/** The status box and its words under the lede. None while the Integration is off and set up: the switch says Off. */
function StatusBlock({ store, summary }: { store: DetailStore; summary: IntegrationSummary }) {
  const usage = useStore(store, (s) => s.usage);
  const devices = useDevices((s) => s.devices);
  const now = useNow(5000);
  const status = statusOf(summary);
  const failed = summary.status.delivery.filter((d) => d.error !== null);
  if (status.tone === null && failed.length === 0) return null;
  return (
    <div className="detail-status">
      {status.tone && (
        <span className="status" data-testid="detail-status" data-state={summary.status.state} data-tone={status.tone}>
          <i className="status-box" data-tone={status.tone} aria-hidden />
          <span className="status-words">{headerLine(status, usage, now)}</span>
        </span>
      )}
      {status.detail && (
        <details className="status-details">
          <summary>Details</summary>
          <p data-testid="detail-status-detail">{status.detail}</p>
        </details>
      )}
      {failed.map((delivery) => (
        <details className="status-details" key={delivery.device} data-testid="delivery-error" data-device={delivery.device}>
          <summary>Couldn't send values to {devices.find((d) => d.id === delivery.device)?.name ?? delivery.device}. See Details.</summary>
          <p>{delivery.error}</p>
        </details>
      ))}
    </div>
  );
}

/** What the last save, secret or check said, under the part of the page it came from. */
export function Outcome({ store, at }: { store: DetailStore; at: ReturnType<typeof outcomeAt> }) {
  const notice = useStore(store, (s) => (s.notice && outcomeAt(s.notice.event) === at ? s.notice : null));
  return (
    <div className={clsx('outcome', notice?.kind === 'error' && 'is-error')} role="status" data-testid="outcome" data-at={at} data-event={notice?.event} data-kind={notice?.kind}>
      {notice && (
        <>
          <span>{notice.text}</span>
          {notice.kind === 'error' && notice.detail && (
            <details className="outcome-details">
              <summary>Details</summary>
              <p>{notice.detail}</p>
            </details>
          )}
        </>
      )}
    </div>
  );
}

function Refresh({ store, summary }: { store: DetailStore; summary: IntegrationSummary }) {
  const busy = useStore(store, (s) => s.busy);
  const after = useStore(store, (s) => s.refreshAfter);
  const [, wake] = useState(0);
  const waiting = after > Date.now();
  useEffect(() => {
    if (!waiting) return;
    const timer = setTimeout(() => wake((n) => n + 1), after - Date.now());
    return () => clearTimeout(timer);
  }, [after, waiting]);
  const usable = summary.enabled && summary.status.state !== 'not_configured';
  return (
    <div className="form-actions">
      <button
        type="button"
        className={button()}
        aria-disabled={!usable || waiting || busy !== null}
        onClick={() => usable && !waiting && void store.refresh()}
        data-testid="refresh"
        data-waiting={waiting}
      >
        {busy === 'refresh' ? 'Checking…' : 'Check now'}
      </button>
      <span className="form-state">{usable ? refreshFloorText(summary.id) : summary.enabled ? 'Finish setup to check.' : `Turn on ${summary.name} to check.`}</span>
    </div>
  );
}

/**
 * One Integration's settings page: its switch, its settings with their own Save, each secret with its own Save, its
 * checks and its accounts. A switch turned on before setup turns the Integration on once a save sets it up.
 */
export function IntegrationDetail({ summary, setUp, onBack }: { summary: IntegrationSummary; setUp: boolean; onBack(): void }) {
  const companion = useCompanion();
  const writes = writesFor(companion);
  const [store] = useState(() => createDetailStore(summary.id, { companion, writes, onSummary: (next) => integrationsStore.getState().upsert(next) }));
  const loaded = useStore(store, (s) => s.view !== null);
  const fields = useStore(store, (s) => s.fields);
  const secretFields = useStore(store, (s) => s.secretFields);
  const usage = useStore(store, (s) => s.usage);
  const switching = useWrites(writes, (s) => s.saving[summary.id] !== undefined);
  const pending = useUi((s) => s.pendingEnable === summary.id);
  const setPendingEnable = useUi((s) => s.setPendingEnable);
  const state = useIntegrationSwitch(summary);
  const title = useRef<HTMLHeadingElement>(null);
  const sheet = useRef<HTMLDivElement>(null);
  const enabling = useRef(false);
  const ids = useId();

  useEffect(() => {
    void store.load();
  }, [store, summary]);

  useEffect(() => {
    if (!loaded) return;
    if (setUp) sheet.current?.querySelector<HTMLElement>('input, textarea')?.focus();
    else title.current?.focus({ preventScroll: true });
  }, [loaded, setUp]);

  // A save took the Integration out of "not set up" while its switch waited: turn it on now.
  useEffect(() => {
    if (!completesPending(summary, pending) || enabling.current) return;
    enabling.current = true;
    void writes.setEnabled(summary.id, true).then((done) => {
      enabling.current = false;
      setPendingEnable(null);
      if (!done) toast.error(`Couldn't turn on ${summary.name}.`);
    });
  }, [summary, pending, writes, setPendingEnable]);

  return (
    <SettingsPage
      id={summary.id}
      name={summary.name}
      icon={summary.icon}
      lede={lineOf(summary.id)}
      status={<StatusBlock store={store} summary={summary} />}
      state={state}
      titleRef={title}
      onBack={onBack}
    >
      <Outcome store={store} at="page" />
      {loaded && (
        <div className="detail-sheet" ref={sheet}>
          {fields.length > 0 && (
            <section className="detail-block" aria-labelledby={`${ids}-settings`} data-testid="detail-settings">
              <h2 className="detail-section-title" id={`${ids}-settings`}>
                {settingsTitle(summary.id)}
              </h2>
              <ConfigForm store={store} switching={switching} outcome={<Outcome store={store} at="settings" />} />
            </section>
          )}
          {secretFields.length > 0 && (
            <section className="detail-block detail-secrets" aria-label={secretFields.map((f) => f.label).join(', ')} data-testid="detail-secrets">
              {secretFields.map((field) => (
                <SecretField key={field.name} store={store} field={field} integration={summary.name} />
              ))}
              <Outcome store={store} at="secret" />
            </section>
          )}
          {summary.actions.includes('refresh') && (
            <section className="detail-block" aria-labelledby={`${ids}-status`} data-testid="detail-checks">
              <h2 className="detail-section-title" id={`${ids}-status`}>
                Status
              </h2>
              <Refresh store={store} summary={summary} />
              <Outcome store={store} at="status" />
            </section>
          )}
        </div>
      )}
      {usage && usage.accounts.length > 0 && (
        <section className="detail-section" aria-labelledby={`${ids}-usage`}>
          <h2 className="detail-section-title" id={`${ids}-usage`}>
            Accounts
          </h2>
          <UsageTable usage={usage} />
        </section>
      )}
    </SettingsPage>
  );
}

