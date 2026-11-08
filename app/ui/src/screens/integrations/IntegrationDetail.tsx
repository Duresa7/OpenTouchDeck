import { useEffect, useRef, useState } from 'react';
import clsx from 'clsx';
import { ChevronLeft } from 'lucide-react';
import { useStore } from 'zustand';
import type { IntegrationSummary, PresetTile } from '../../lib/companion/api.ts';
import { useCompanion } from '../../lib/companion/connection.ts';
import { useDevices } from '../../lib/companion/devices.ts';
import { createDetailStore, refreshFloorText, type DetailStore } from '../../lib/integrations/detail.ts';
import { lineOf, statusOf, usageLine } from '../../lib/integrations/model.ts';
import { button } from '../../shell/button.ts';
import { ConfigForm } from './ConfigForm.tsx';
import { usePresetNeeds } from './addToLayout.ts';
import { IntegrationGlyph, PresetButton } from './PresetPreview.tsx';
import { SecretField } from './SecretField.tsx';
import { UsageTable } from './UsageTable.tsx';

function useNow(everyMs: number) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), everyMs);
    return () => clearInterval(timer);
  }, [everyMs]);
  return now;
}

function StatusBlock({ store, summary }: { store: DetailStore; summary: IntegrationSummary }) {
  const usage = useStore(store, (s) => s.usage);
  const devices = useDevices((s) => s.devices);
  const now = useNow(5000);
  const status = statusOf(summary);
  const words = status.live && summary.enabled && usage ? usageLine(usage, now) : status.words;
  const failed = summary.status.delivery.filter((d) => d.error !== null);
  return (
    <div className="detail-status">
      <span className="status" data-testid="detail-status" data-state={summary.status.state} data-live={status.live}>
        {status.tone && <i className="status-box" data-tone={status.tone} aria-hidden />}
        {words}
      </span>
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
      <span className="form-state">{refreshFloorText(summary.id)}</span>
    </div>
  );
}

function Outcome({ store }: { store: DetailStore }) {
  const notice = useStore(store, (s) => s.notice);
  return (
    <div className={clsx('outcome', notice?.kind === 'error' && 'is-error')} role="status" data-testid="outcome" data-event={notice?.event} data-kind={notice?.kind}>
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

/** One Integration: its status, the settings its schema builds, its secrets, Refresh, its Preset tiles and usage. */
export function IntegrationDetail({ summary, setUp, onBack, onAdd }: { summary: IntegrationSummary; setUp: boolean; onBack(): void; onAdd(preset: PresetTile): void }) {
  const companion = useCompanion();
  const needs = usePresetNeeds();
  const [store] = useState(() => createDetailStore(summary.id, { companion }));
  const loaded = useStore(store, (s) => s.view !== null);
  const secretFields = useStore(store, (s) => s.secretFields);
  const usage = useStore(store, (s) => s.usage);
  const title = useRef<HTMLHeadingElement>(null);
  const sheet = useRef<HTMLDivElement>(null);

  useEffect(() => {
    void store.load();
  }, [store, summary]);

  useEffect(() => {
    if (!loaded) return;
    if (setUp) sheet.current?.querySelector<HTMLElement>('input, [role="switch"]')?.focus();
    else title.current?.focus({ preventScroll: true });
  }, [loaded, setUp]);

  return (
    <div className="integration-detail" data-testid="integration-detail" data-id={summary.id}>
      <button type="button" className={clsx(button({ size: 'small' }), 'detail-back')} onClick={onBack} data-testid="detail-back">
        <ChevronLeft size={16} strokeWidth={1.9} aria-hidden />
        Integrations
      </button>
      <header className="detail-head">
        <IntegrationGlyph icon={summary.icon} size={22} />
        <div className="detail-head-text">
          <h1 className="page-title detail-title" ref={title} tabIndex={-1}>
            {summary.name}
          </h1>
          <p className="page-lede">{lineOf(summary.id)}</p>
          <StatusBlock store={store} summary={summary} />
        </div>
      </header>
      {loaded && (
        <div className="detail-sheet" ref={sheet}>
          <ConfigForm store={store} />
          {secretFields.map((field) => (
            <SecretField key={field.name} store={store} field={field} integration={summary.name} />
          ))}
          {summary.actions.includes('refresh') && <Refresh store={store} summary={summary} />}
          <Outcome store={store} />
        </div>
      )}
      {summary.presets.length > 0 && (
        <section className="detail-section" aria-labelledby="presets-title">
          <h2 className="detail-section-title" id="presets-title">
            Preset tiles
          </h2>
          <div className="preset-grid">
            {summary.presets.map((preset) => (
              <PresetButton key={preset.id} preset={preset} streams={summary.streams} onAdd={onAdd} needs={needs(preset)} />
            ))}
          </div>
        </section>
      )}
      {usage && usage.accounts.length > 0 && (
        <section className="detail-section" aria-labelledby="usage-title">
          <h2 className="detail-section-title" id="usage-title">
            Accounts
          </h2>
          <UsageTable usage={usage} />
        </section>
      )}
    </div>
  );
}
