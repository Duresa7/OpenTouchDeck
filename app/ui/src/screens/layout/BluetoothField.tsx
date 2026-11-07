import { useEffect, useId, useState } from 'react';
import clsx from 'clsx';
import { useStore } from 'zustand';
import { Switch } from '@base-ui/react/switch';
import type { DeviceSummary } from '../../lib/companion/api.ts';
import { bluetoothStatusText, countdown, offersBluetooth, type BluetoothStore } from '../../lib/device/bluetooth.ts';
import { needsFirmwareField } from '../../lib/device/needs-firmware.ts';
import { button } from '../../shell/button.ts';

interface BluetoothFieldProps {
  device: DeviceSummary;
  name: string;
  bluetooth: BluetoothStore;
  /** Connected and not restarting. */
  live: boolean;
}

/** The radio build's Bluetooth keyboard: on and off, the window for adding a computer or phone, and the Keyboard hosts
 * it keeps. It changes only over USB, so a Device on Wi-Fi shows it read-only. */
export function BluetoothField({ device, name, bluetooth, live }: BluetoothFieldProps) {
  const id = useId();
  const shown = useStore(bluetooth, (s) => s.bluetooth);
  const busy = useStore(bluetooth, (s) => s.busy);
  const forgetting = useStore(bluetooth, (s) => s.forgetting);
  const needs = useStore(bluetooth, (s) => s.needs);
  const error = useStore(bluetooth, (s) => s.error);
  const [confirming, setConfirming] = useState(false);
  const offered = offersBluetooth(device);
  const overUsb = live && device.carrier !== 'wifi';

  useEffect(() => {
    if (!overUsb || !offered) return;
    void bluetooth.load();
    return () => bluetooth.pause();
  }, [bluetooth, overUsb, offered]);

  const bonds = shown?.bonds ?? [];
  useEffect(() => {
    if (bonds.length === 0) setConfirming(false);
  }, [bonds.length]);

  if (!offered) return null;
  if (needs) {
    return (
      <div className="field" data-testid="device-panel-bluetooth">
        <span className="field-label">Bluetooth keyboard</span>
        <p className="field-hint is-warning" data-testid="device-panel-bluetooth-status">
          {needsFirmwareField({ device: name, since: null })}
        </p>
      </div>
    );
  }

  const editable = overUsb && shown !== null && busy === null;
  const on = shown?.state === 'on';
  const full = shown !== null && bonds.length >= shown.bonds_max;
  const status = !live
    ? `Plug in ${name} to change its Bluetooth keyboard.`
    : !overUsb
      ? `Plug in ${name} with a USB cable to change its Bluetooth keyboard.`
      : busy === 'switching'
        ? shown?.enabled
          ? 'Turning off…'
          : 'Turning on…'
        : shown
          ? bluetoothStatusText(shown, name)
          : 'Reading…';
  const act = (go: () => Promise<boolean>) => () => {
    if (editable) void go();
  };

  return (
    <div className="field panel-bluetooth" data-testid="device-panel-bluetooth" data-state={shown?.state} data-usb={overUsb}>
      <div className="field-label field-label-row switch-row">
        <span id={`${id}-title`}>Bluetooth keyboard</span>
        <Switch.Root
          className="switch"
          checked={shown?.enabled ?? false}
          disabled={!editable}
          onCheckedChange={(enabled) => void bluetooth.setEnabled(enabled)}
          aria-labelledby={`${id}-title`}
          aria-describedby={`${id}-status`}
          data-testid="device-panel-bluetooth-switch"
        >
          <Switch.Thumb className="switch-thumb" />
        </Switch.Root>
      </div>
      <p
        id={`${id}-status`}
        className={clsx('field-hint', on && (shown?.pairing ? shown.passkey : bonds.some((b) => b.connected)) && 'is-ok')}
        role="status"
        data-testid="device-panel-bluetooth-status"
      >
        {status}
      </p>
      {on && shown?.pairing && (
        <div className="panel-confirm" role="group" aria-labelledby={`${id}-window`} data-testid="device-panel-bluetooth-window" data-passkey={shown.passkey}>
          <p className="panel-confirm-title field-label-row" id={`${id}-window`}>
            <span>Adding a computer or phone</span>
            <span className="field-value tabular" data-testid="device-panel-bluetooth-countdown" aria-label={`${countdown(shown.pairing_ms)} left`}>
              {countdown(shown.pairing_ms)}
            </span>
          </p>
          <p className="field-hint">
            Open Bluetooth on the computer or phone and choose {name}. {name} shows a 6-digit code; type it on the computer or phone.
          </p>
          <div className="panel-confirm-buttons">
            <button type="button" className={button({ size: 'small' })} aria-disabled={!editable} onClick={act(bluetooth.closePairing)} data-testid="device-panel-bluetooth-stop">
              Stop adding
            </button>
          </div>
        </div>
      )}
      {on && !shown?.pairing && (
        <div className="panel-row-buttons">
          <button
            type="button"
            className={button({ size: 'small' })}
            aria-disabled={!editable || full}
            aria-describedby={full ? `${id}-full` : undefined}
            onClick={() => editable && !full && void bluetooth.openPairing()}
            data-testid="device-panel-bluetooth-add"
          >
            Add a computer or phone
          </button>
        </div>
      )}
      {on && full && !shown?.pairing && (
        <p className="field-hint" id={`${id}-full`} data-testid="device-panel-bluetooth-full">
          {name} keeps {shown.bonds_max} computers and phones. Forget one to add another.
        </p>
      )}
      {/* The Device may list its Keyboard hosts while it starts again, as after a host reset. */}
      {bonds.length > 0 && (
        <ul className="panel-hosts" aria-label="Keyboard hosts" data-testid="device-panel-bluetooth-bonds">
          {bonds.map((bond) => (
            <li key={bond.address} className="panel-row" data-testid="device-panel-bluetooth-bond" data-address={bond.address} data-connected={bond.connected}>
              <span className="row-text">
                <span className="row-title tabular">{bond.address}</span>
                <span className={clsx('row-sub', bond.connected && 'is-ok')}>{bond.connected ? 'Connected' : 'Not connected'}</span>
              </span>
              <button
                type="button"
                className={button({ size: 'small' })}
                aria-disabled={!editable || !on}
                aria-label={`Forget ${bond.address}`}
                onClick={() => editable && on && void bluetooth.forget(bond.address)}
                data-testid="device-panel-bluetooth-forget"
              >
                {forgetting === bond.address ? 'Forgetting…' : 'Forget'}
              </button>
            </li>
          ))}
        </ul>
      )}
      {on && bonds.length > 0 && !confirming && (
        <div className="panel-row-buttons">
          <button
            type="button"
            className={button({ kind: 'danger', size: 'small' })}
            aria-disabled={!editable}
            onClick={() => editable && setConfirming(true)}
            data-testid="device-panel-bluetooth-forget-all"
          >
            Forget all
          </button>
        </div>
      )}
      {on && confirming && (
        <div className="panel-confirm" role="group" aria-labelledby={`${id}-confirm`} data-testid="device-panel-bluetooth-confirm">
          <p className="panel-confirm-title" id={`${id}-confirm`}>
            Forget every computer and phone?
          </p>
          <p className="field-hint">Each one needs a new code from {name} to be added again.</p>
          <div className="panel-confirm-buttons">
            <button type="button" className={button({ size: 'small' })} onClick={() => setConfirming(false)} data-testid="device-panel-bluetooth-confirm-cancel">
              Cancel
            </button>
            <button
              type="button"
              className={button({ kind: 'danger', size: 'small' })}
              aria-disabled={!editable}
              onClick={act(async () => {
                const done = await bluetooth.forgetAll();
                if (done) setConfirming(false);
                return done;
              })}
              data-testid="device-panel-bluetooth-confirm-forget"
            >
              {forgetting === 'all' ? 'Forgetting…' : 'Forget all'}
            </button>
          </div>
        </div>
      )}
      {error && (
        <p className="field-error" role="alert" data-testid="device-panel-bluetooth-error">
          {error}
        </p>
      )}
    </div>
  );
}
