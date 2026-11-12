import { useEffect, useId, useState } from 'react';
import { useStore } from 'zustand';
import type { DeviceSummary } from '../../lib/companion/api.ts';
import { needsFirmwareField } from '../../lib/device/needs-firmware.ts';
import { offersPairings, pairedCount, type DevicePairingsStore } from '../../lib/device/pairings.ts';
import { button } from '../../shell/button.ts';

interface PairingsFieldProps {
  device: DeviceSummary;
  name: string;
  pairings: DevicePairingsStore;
  /** Connected and not restarting. */
  live: boolean;
}

/** The apps a radio-build Device keeps Pairings with, read and forgotten over USB. Forget all asks first. */
export function PairingsField({ device, name, pairings, live }: PairingsFieldProps) {
  const id = useId();
  const shown = useStore(pairings, (s) => s.pairings);
  const forgetting = useStore(pairings, (s) => s.forgetting);
  const needs = useStore(pairings, (s) => s.needs);
  const error = useStore(pairings, (s) => s.error);
  const [confirming, setConfirming] = useState(false);
  const offered = offersPairings(device);
  const overUsb = live && device.carrier !== 'wifi';

  useEffect(() => {
    if (overUsb && offered) void pairings.load();
  }, [pairings, overUsb, offered]);

  const apps = shown?.pairings ?? [];
  useEffect(() => {
    if (apps.length === 0) setConfirming(false);
  }, [apps.length]);

  if (!offered) return null;
  if (needs) {
    return (
      <div className="field" data-testid="device-panel-pairings">
        <span className="field-label">Paired apps</span>
        <p className="field-hint is-warning" data-testid="device-panel-pairings-status">
          {needsFirmwareField({ device: name, since: null })}
        </p>
      </div>
    );
  }

  const editable = overUsb && shown !== null && forgetting === null;
  const full = shown !== null && apps.length >= shown.max;
  const status = !live
    ? `Plug in ${name} to see its paired apps.`
    : !overUsb
      ? `Plug in ${name} with a USB cable to see its paired apps.`
      : shown === null
        ? 'Reading…'
        : apps.length === 0
          ? `No apps are paired. Pair one over Wi-Fi to use ${name} without the cable.`
          : full
            ? 'Four apps are paired. Forget one to pair another.'
            : null;

  return (
    <div className="field panel-pairings" data-testid="device-panel-pairings" data-usb={overUsb}>
      <div className="field-label field-label-row">
        <span id={`${id}-title`}>Paired apps</span>
        {overUsb && shown && (
          <span className="field-value tabular" data-testid="device-panel-pairings-count">
            {pairedCount(shown)}
          </span>
        )}
      </div>
      {status && (
        <p className="field-hint" role="status" data-testid="device-panel-pairings-status">
          {status}
        </p>
      )}
      {overUsb && apps.length > 0 && (
        <ul className="panel-hosts" aria-labelledby={`${id}-title`} data-testid="device-panel-pairings-list">
          {apps.map((app) => (
            <li key={app.app} className="panel-row" data-testid="device-panel-pairing" data-app={app.app} data-this-app={app.this_app}>
              <span className="row-text">
                <span className="row-title">{app.name}</span>
                {app.this_app && <span className="row-sub">This app</span>}
              </span>
              <button
                type="button"
                className={button({ size: 'small' })}
                aria-disabled={!editable}
                aria-label={`Forget ${app.name}`}
                onClick={() => editable && void pairings.forget(app.app)}
                data-testid="device-panel-pairing-forget"
              >
                {forgetting === app.app ? 'Forgetting…' : 'Forget'}
              </button>
            </li>
          ))}
        </ul>
      )}
      {overUsb && apps.length > 0 && !confirming && (
        <div className="panel-row-buttons">
          <button
            type="button"
            className={button({ kind: 'danger', size: 'small' })}
            aria-disabled={!editable}
            onClick={() => editable && setConfirming(true)}
            data-testid="device-panel-pairings-forget-all"
          >
            Forget all
          </button>
        </div>
      )}
      {overUsb && confirming && (
        <div className="panel-confirm" role="group" aria-labelledby={`${id}-confirm`} data-testid="device-panel-pairings-confirm">
          <p className="panel-confirm-title" id={`${id}-confirm`}>
            Forget every paired app?
          </p>
          <p className="field-hint">None of them can reach {name} over Wi-Fi until it pairs again with a new code.</p>
          <div className="panel-confirm-buttons">
            <button type="button" className={button({ size: 'small' })} onClick={() => setConfirming(false)} data-testid="device-panel-pairings-confirm-cancel">
              Cancel
            </button>
            <button
              type="button"
              className={button({ kind: 'danger', size: 'small' })}
              aria-disabled={!editable}
              onClick={() => {
                if (!editable) return;
                void pairings.forgetAll().then((done) => done && setConfirming(false));
              }}
              data-testid="device-panel-pairings-confirm-forget"
            >
              {forgetting === 'all' ? 'Forgetting…' : 'Forget all'}
            </button>
          </div>
        </div>
      )}
      {error && (
        <p className="field-error" role="alert" data-testid="device-panel-pairings-error">
          {error}
        </p>
      )}
    </div>
  );
}
