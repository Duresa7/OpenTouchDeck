import { useEffect, useId, useState, type FormEvent } from 'react';
import clsx from 'clsx';
import { useStore } from 'zustand';
import type { DeviceSummary } from '../../lib/companion/api.ts';
import { needsFirmwareField } from '../../lib/device/needs-firmware.ts';
import { offersWifiSetup, wifiSetupStatusText, type WifiSetupStore } from '../../lib/device/wifi-setup.ts';
import { button } from '../../shell/button.ts';

interface WifiSetupFieldProps {
  device: DeviceSummary;
  name: string;
  setup: WifiSetupStore;
  /** Connected and not restarting. */
  live: boolean;
}

/** The network the Device joins, set over USB on a Device whose Firmware has a Wi-Fi station. The password field
 * empties after every join, and the password is never read back. */
export function WifiSetupField({ device, name, setup, live }: WifiSetupFieldProps) {
  const id = useId();
  const shown = useStore(setup, (s) => s.wifi);
  const busy = useStore(setup, (s) => s.busy);
  const problem = useStore(setup, (s) => s.problem);
  const needs = useStore(setup, (s) => s.needs);
  const error = useStore(setup, (s) => s.error);
  const [ssid, setSsid] = useState('');
  const [password, setPassword] = useState('');
  const offered = offersWifiSetup(device);
  const overUsb = live && device.carrier !== 'wifi';
  const stored = shown?.ssid ?? '';

  useEffect(() => {
    if (overUsb && offered) void setup.load();
  }, [setup, overUsb, offered]);
  useEffect(() => {
    if (stored) setSsid((draft) => draft || stored);
  }, [stored]);

  if (!offered) return null;
  if (needs) {
    return (
      <div className="panel-wifi" data-testid="device-panel-wifi-setup">
        <span className="row-title">Network</span>
        <p className="field-hint is-warning" data-testid="device-panel-wifi-setup-status">
          {needsFirmwareField({ device: name, since: null })}
        </p>
      </div>
    );
  }

  const editable = overUsb && shown !== null && busy === null;
  const join = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!editable) return;
    if (await setup.join(ssid, password)) setPassword('');
  };
  const status = !live
    ? `Plug in ${name} to change its Wi-Fi network.`
    : !overUsb
      ? `Plug in ${name} with a USB cable to change its Wi-Fi network.`
      : busy === 'joining'
        ? `Sending to ${name}…`
        : busy === 'forgetting'
          ? 'Forgetting…'
          : shown
            ? wifiSetupStatusText(shown, name)
            : 'Reading…';
  return (
    <form
      className="panel-wifi"
      aria-labelledby={`${id}-title`}
      onSubmit={(event) => void join(event)}
      noValidate
      data-testid="device-panel-wifi-setup"
      data-state={shown?.state}
    >
      <span className="row-text">
        <span className="row-title" id={`${id}-title`}>
          Network
        </span>
        <span
          className={clsx('row-sub', shown?.state === 'connected' && overUsb && 'is-ok')}
          role="status"
          data-testid="device-panel-wifi-setup-status"
          data-state={shown?.state}
          data-reason={shown?.reason ?? undefined}
        >
          {status}
        </span>
      </span>
      <label className="field-label" htmlFor={`${id}-ssid`}>
        Network name
      </label>
      <input
        id={`${id}-ssid`}
        className="input"
        value={ssid}
        disabled={!overUsb}
        readOnly={!editable}
        spellCheck={false}
        autoComplete="off"
        aria-invalid={problem.ssid !== undefined}
        aria-describedby={problem.ssid ? `${id}-ssid-error` : undefined}
        onChange={(event) => setSsid(event.target.value)}
        data-testid="device-panel-wifi-setup-ssid"
      />
      {problem.ssid && (
        <p id={`${id}-ssid-error`} className="field-error" data-testid="device-panel-wifi-setup-ssid-error">
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
        disabled={!overUsb}
        readOnly={!editable}
        spellCheck={false}
        autoComplete="new-password"
        aria-invalid={problem.password !== undefined}
        aria-describedby={`${id}-password-hint`}
        onChange={(event) => setPassword(event.target.value)}
        data-testid="device-panel-wifi-setup-password"
      />
      <p id={`${id}-password-hint`} className={problem.password ? 'field-error' : 'field-hint'} data-testid="device-panel-wifi-setup-password-hint">
        {problem.password ?? `Empty for an open network. Stored on ${name} only, and never shown again.`}
      </p>
      <div className="panel-row-buttons panel-wifi-buttons">
        {shown?.configured && (
          <button
            type="button"
            className={button({ size: 'small' })}
            aria-disabled={!editable}
            onClick={() => {
              if (editable) void setup.forget().then((forgot) => forgot && setSsid(''));
            }}
            data-testid="device-panel-wifi-setup-forget"
          >
            Forget network
          </button>
        )}
        <button type="submit" className={button({ kind: 'primary', size: 'small' })} aria-disabled={!editable} data-testid="device-panel-wifi-setup-join">
          Join
        </button>
      </div>
      {error && (
        <p className="field-error" role="alert" data-testid="device-panel-wifi-setup-error">
          {error}
        </p>
      )}
    </form>
  );
}
