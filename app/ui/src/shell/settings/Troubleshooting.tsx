import { useEffect, useId, useRef } from 'react';
import clsx from 'clsx';
import { Dialog } from '@base-ui/react/dialog';
import { ChevronLeft, X } from 'lucide-react';
import type { DeviceSummary } from '../../lib/companion/api.ts';
import { useDevices } from '../../lib/companion/devices.ts';
import { statusOf } from '../../screens/home-model.ts';
import { button } from '../button.ts';
import { StatusBox } from '../StatusBox.tsx';
import { FirmwareConsole } from './FirmwareConsole.tsx';
import { FirmwareFile } from './FirmwareFile.tsx';
import { useSystemInfo } from './system.ts';
import { connectionText, factsOf, lastError, nameOf } from './troubleshooting.ts';

function DeviceDetails({ device, demo }: { device: DeviceSummary; demo: boolean }) {
  const status = statusOf(device.link.state);
  const error = lastError(device);
  return (
    <div className="group" data-testid="device-details" data-device={device.id}>
      <div className="row">
        <span className="row-text">
          <span className="row-title">{nameOf(device)}</span>
          <span className="row-sub">{connectionText({ device, demo })}</span>
          {error && (
            <details className="row-details">
              <summary>Last connection error</summary>
              <p>{error}</p>
            </details>
          )}
        </span>
        <StatusBox tone={status.tone} label={status.words} showWords />
      </div>
      {device.link.state === 'connected' && (
        <dl className="facts">
          {factsOf(device).map((fact) => (
            <div className="fact" key={fact.key} data-fact={fact.key}>
              <dt>{fact.label}</dt>
              <dd className={clsx(fact.value === null && 'is-unknown')}>{fact.value ?? 'Unknown'}</dd>
            </div>
          ))}
        </dl>
      )}
    </div>
  );
}

/** #83 moves the Key test, the Action log and the always-allowed list here. */
function KeyTestSlot() {
  return null;
}

/** #77 adds Firmware repair here. */
function FirmwareRepairSlot() {
  return null;
}

export function TroubleshootingView({ onBack, focusBack }: { onBack(): void; focusBack: boolean }) {
  const back = useRef<HTMLButtonElement>(null);
  const devices = useDevices((s) => s.devices);
  const info = useSystemInfo();
  const demo = info !== null && info !== 'unknown' && info.mock;
  const id = useId();
  useEffect(() => {
    if (focusBack) back.current?.focus();
  }, [focusBack]);

  return (
    <>
      <div className="dialog-head">
        <div className="dialog-head-start">
          <button ref={back} type="button" className={button({ kind: 'icon' })} aria-label="Back" onClick={onBack}>
            <ChevronLeft size={18} strokeWidth={1.9} aria-hidden />
          </button>
          <Dialog.Title className="dialog-title">Troubleshooting</Dialog.Title>
        </div>
        <Dialog.Close className={button({ kind: 'icon' })} aria-label="Close">
          <X size={18} strokeWidth={1.9} aria-hidden />
        </Dialog.Close>
      </div>
      <p className="dialog-lede">Device details, and tools to check that taps reach this PC.</p>

      <section className="settings-section" aria-labelledby={id}>
        <h3 id={id}>Connection</h3>
        <div className="groups">
          {devices.length ? (
            devices.map((device) => <DeviceDetails key={device.id} device={device} demo={demo} />)
          ) : (
            <div className="group">
              <div className="row">
                <span className="row-note">No Device connected. Plug one in with a USB cable.</span>
              </div>
            </div>
          )}
        </div>
      </section>
      <FirmwareConsole devices={devices} />
      <KeyTestSlot />
      <FirmwareFile devices={devices} />
      <FirmwareRepairSlot />
    </>
  );
}
