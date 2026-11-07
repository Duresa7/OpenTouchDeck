import { useEffect, useId, useState, type FormEvent } from 'react';
import clsx from 'clsx';
import { useStore } from 'zustand';
import type { DeviceSummary } from '../../lib/companion/api.ts';
import { needsFirmwareField } from '../../lib/device/needs-firmware.ts';
import { offersWifi, wifiStatusText, type DeviceWifiStore } from '../../lib/device/wifi.ts';
import { button } from '../../shell/button.ts';

interface WifiFieldProps {
  device: DeviceSummary;
  name: string;
  wifi: DeviceWifiStore;
  live: boolean;
}

/** The Device's Wi-Fi network, on a Device whose Firmware has a Wi-Fi station. The password field empties after
 * every join, and the password is never read back. */
export function WifiField({ device, name, wifi, live }: WifiFieldProps) {
  const id = useId();
  const shown = useStore(wifi, (s) => s.wifi);
  const busy = useStore(wifi, (s) => s.busy);
  const problem = useStore(wifi, (s) => s.problem);
  const needs = useStore(wifi, (s) => s.needs);
  const error = useStore(wifi, (s) => s.error);
  const [ssid, setSsid] = useState('');
  const [password, setPassword] = useState('');
  const offered = offersWifi(device);
  const stored = shown?.ssid ?? '';

  useEffect(() => {
    if (live && offered) void wifi.load();
  }, [wifi, live, offered]);
  useEffect(() => {
    if (stored) setSsid((draft) => draft || stored);
  }, [stored]);

  if (!offered) return null;
  if (needs) {
    return (
      <div className="field" data-testid="device-panel-wifi">
        <span className="field-label">Wi-Fi</span>
        <p className="field-hint is-warning" data-testid="device-panel-wifi-status">
          {needsFirmwareField({ device: name, since: null })}
        </p>
      </div>
    );
  }

  const editable = live && shown !== null && busy === null;
  const join = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!editable) return;
    if (await wifi.join(ssid, password)) setPassword('');
  };
  const status = shown ? wifiStatusText(shown, name) : live ? 'Reading…' : `Plug in ${name} to change its Wi-Fi.`;
  return (
    <div className="field" data-testid="device-panel-wifi" data-state={shown?.state}>
      <span className="field-label" id={`${id}-title`}>
        Wi-Fi
      </span>
      <p
        className={clsx('field-hint', shown?.state === 'connected' && 'is-ok', shown?.state === 'failed' && 'is-warning')}
        role="status"
        data-testid="device-panel-wifi-status"
        data-state={shown?.state}
        data-reason={shown?.reason ?? undefined}
      >
        {busy === 'joining' ? `Sending to ${name}…` : busy === 'forgetting' ? 'Forgetting…' : status}
      </p>
      <form className="panel-wifi" aria-labelledby={`${id}-title`} onSubmit={(event) => void join(event)} noValidate>
        <label className="field-label" htmlFor={`${id}-ssid`}>
          Network name
        </label>
        <input
          id={`${id}-ssid`}
          className="input"
          value={ssid}
          disabled={!live}
          readOnly={!editable}
          spellCheck={false}
          autoComplete="off"
          aria-invalid={problem.ssid !== undefined}
          aria-describedby={problem.ssid ? `${id}-ssid-error` : undefined}
          onChange={(event) => setSsid(event.target.value)}
          data-testid="device-panel-wifi-ssid"
        />
        {problem.ssid && (
          <p id={`${id}-ssid-error`} className="field-error" data-testid="device-panel-wifi-ssid-error">
            {problem.ssid}
          </p>
        )}
        <label className="field-label" htmlFor={`${id}-password`}>
          Password
        </label>
        <input
          id={`${id}-password`}
          className="input"
          type="password"
          value={password}
          disabled={!live}
          readOnly={!editable}
          spellCheck={false}
          autoComplete="new-password"
          aria-invalid={problem.password !== undefined}
          aria-describedby={`${id}-password-hint`}
          onChange={(event) => setPassword(event.target.value)}
          data-testid="device-panel-wifi-password"
        />
        <p id={`${id}-password-hint`} className={problem.password ? 'field-error' : 'field-hint'} data-testid="device-panel-wifi-password-hint">
          {problem.password ?? `Empty for an open network. Stored on ${name} only, and never shown again.`}
        </p>
        <div className="panel-row-buttons panel-wifi-buttons">
          {shown?.configured && (
            <button
              type="button"
              className={button({ size: 'small' })}
              aria-disabled={!editable}
              onClick={() => {
                if (editable) void wifi.forget().then((forgot) => forgot && setSsid(''));
              }}
              data-testid="device-panel-wifi-forget"
            >
              Forget
            </button>
          )}
          <button type="submit" className={button({ kind: 'primary', size: 'small' })} aria-disabled={!editable} data-testid="device-panel-wifi-join">
            Join
          </button>
        </div>
      </form>
      {error && (
        <p className="field-error" role="alert" data-testid="device-panel-wifi-error">
          {error}
        </p>
      )}
    </div>
  );
}
