import { useId, useLayoutEffect, useRef, type RefObject } from 'react';
import { Dialog } from '@base-ui/react/dialog';
import { useDevices } from '../lib/companion/devices.ts';
import { useUpdates, type UpdateOffer, type UpdateStep } from '../state/updates.ts';
import { button } from './button.ts';
import { bridgeReadyText, bridgeText, changeLines, downloadingText, failureText, percent, sizeText, waitText } from './update-text.ts';

type Downloading = Extract<UpdateStep, { kind: 'downloading' }>;

function Offer({ offer }: { offer: UpdateOffer }) {
  const ids = useId();
  const lines = changeLines(offer.notes);
  return (
    <>
      <Dialog.Title className="dialog-title">Open TouchDeck {offer.version}</Dialog.Title>
      <Dialog.Description className="update-text">{offer.bridge ? bridgeText(offer) : `You have ${offer.current}.`}</Dialog.Description>
      <h3 className="update-changes-heading" id={`${ids}-changes`}>
        What's new
      </h3>
      {lines.length > 0 ? (
        <ul className="update-changes" aria-labelledby={`${ids}-changes`}>
          {lines.map((line, at) => (
            <li key={at}>{line}</li>
          ))}
        </ul>
      ) : (
        <p className="update-changes update-changes-none">No release notes.</p>
      )}
    </>
  );
}

function Progress({ step, labelledBy }: { step: Downloading; labelledBy: string }) {
  const pct = percent(step);
  const size = sizeText(step);
  return (
    <>
      <div
        className="update-progress"
        role="progressbar"
        aria-labelledby={labelledBy}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pct ?? undefined}
        aria-valuetext={downloadingText(step)}
      >
        <span style={{ transform: `scaleX(${(pct ?? 0) / 100})` }} />
      </div>
      <p className="update-progress-meta">
        <span>{size ?? downloadingText(step)}</span>
        {pct !== null && <span>{pct}%</span>}
      </p>
    </>
  );
}

function useDeviceName(device: string | null): string | null {
  return useDevices((s) => {
    const known = s.devices.find((d) => d.id === device);
    return known ? known.name : device;
  });
}

function readyText(step: UpdateStep, deviceName: string | null, offer: UpdateOffer): string {
  if (step.kind === 'busy') return waitText(deviceName);
  if (step.kind === 'installing') return 'Restarting to install…';
  return offer.bridge ? bridgeReadyText(offer.version) : 'Open TouchDeck restarts to install. Your Layouts and settings stay.';
}

function useKeepFocusInDialog(popup: RefObject<HTMLDivElement | null>, stepKind: UpdateStep['kind']) {
  useLayoutEffect(() => {
    const dialog = popup.current;
    if (dialog && !dialog.contains(document.activeElement)) dialog.focus({ preventScroll: true });
  }, [popup, stepKind]);
}

export function UpdateDialog({ update }: { update: { offer: UpdateOffer; step: UpdateStep } }) {
  const popup = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const download = useUpdates((s) => s.download);
  const install = useUpdates((s) => s.install);
  const retry = useUpdates((s) => s.retry);
  const later = useUpdates((s) => s.later);
  const { offer, step } = update;
  const deviceName = useDeviceName(step.kind === 'busy' ? step.device : null);
  useKeepFocusInDialog(popup, step.kind);

  const laterButton = (
    <button type="button" className={button()} onClick={later}>
      Later
    </button>
  );

  return (
    <Dialog.Portal>
      <Dialog.Backdrop className="scrim" />
      <Dialog.Popup ref={popup} className="dialog dialog-update" initialFocus={(type) => (type === 'keyboard' ? true : popup.current)}>
        {step.kind === 'offer' && (
          <>
            <Offer offer={offer} />
            <div className="update-actions">
              {laterButton}
              <button type="button" className={button({ kind: 'primary' })} onClick={() => void download()}>
                Update
              </button>
            </div>
          </>
        )}

        {step.kind === 'downloading' && (
          <>
            <Dialog.Title className="dialog-title" id={titleId}>
              Open TouchDeck {offer.version}
            </Dialog.Title>
            <Dialog.Description className="update-text">You can keep working.</Dialog.Description>
            <Progress step={step} labelledBy={titleId} />
            <div className="update-actions">
              <Dialog.Close className={button()}>Hide</Dialog.Close>
            </div>
          </>
        )}

        {(step.kind === 'ready' || step.kind === 'busy' || step.kind === 'installing') && (
          <>
            <Dialog.Title className="dialog-title">Ready to restart</Dialog.Title>
            <Dialog.Description className="update-text" aria-live="polite">
              {readyText(step, deviceName, offer)}
            </Dialog.Description>
            <div className="update-actions">
              {laterButton}
              <button
                type="button"
                className={button({ kind: 'primary' })}
                aria-disabled={step.kind === 'installing' || undefined}
                onClick={() => void install()}
              >
                Restart
              </button>
            </div>
          </>
        )}

        {step.kind === 'failed' && (
          <>
            <Dialog.Title className="dialog-title">Open TouchDeck {offer.version}</Dialog.Title>
            <Dialog.Description className="update-text" role="alert">
              {failureText[step.reason]}
            </Dialog.Description>
            <div className="update-actions">
              {laterButton}
              <button type="button" className={button({ kind: 'primary' })} onClick={() => void retry()}>
                Retry
              </button>
            </div>
          </>
        )}
      </Dialog.Popup>
    </Dialog.Portal>
  );
}
