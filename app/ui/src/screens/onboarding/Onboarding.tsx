import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import clsx from 'clsx';
import { useStore } from 'zustand';
import { Dialog } from '@base-ui/react/dialog';
import { LayoutGrid, Sparkles, Usb, Wifi, X, type LucideIcon } from 'lucide-react';
import type { DeviceSummary } from '../../lib/companion/api.ts';
import { useCompanion } from '../../lib/companion/connection.ts';
import { jobsStore, waitForJob } from '../../lib/companion/jobs.ts';
import { inProgress, type FirmwareUpdateStore } from '../../lib/device/firmware.ts';
import { needsFirmwareField } from '../../lib/device/needs-firmware.ts';
import { createSetup, describeOnDevice, stepCount, type Busy, type LayoutChoice, type SetupState, type SetupStore } from '../../lib/device/onboarding.ts';
import { newLayout, starterLayout, type Layout, type Orientation } from '../../lib/layout/model.ts';
import { button } from '../../shell/button.ts';
import { DevicePreview } from '../DevicePreview.tsx';
import { statusOf } from '../home-model.ts';
import { deviceStoresFor, useOfferedFirmware } from '../layout/device.ts';
import './onboarding.css';

export type CloseThen = 'stay' | 'edit';

const ORIENTATIONS: { value: Orientation; label: string }[] = [
  { value: 'landscape', label: 'Landscape' },
  { value: 'portrait', label: 'Portrait' },
];

const BUSY_LABEL: Record<Busy, string> = {
  saving: 'Saving…',
  restarting: 'Restarting…',
  reading: 'Loading…',
  sending: 'Sending…',
};

function drafts() {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

const opening = (name: string) => name.charAt(0).toUpperCase() + name.slice(1);

function useSetup(device: DeviceSummary): { setup: SetupStore; firmware: FirmwareUpdateStore; offered: string | null } {
  const companion = useCompanion();
  const stores = deviceStoresFor(companion, device);
  const offered = useOfferedFirmware(device);
  const [setup] = useState(() =>
    createSetup({
      companion,
      wait: (job) => waitForJob<'layouts.push'>(jobsStore, companion, job),
      settings: stores.settings,
      drafts: drafts(),
      device,
      firmware: { offered },
    }),
  );
  const job = device.link.state === 'updating' ? device.link.job : null;
  useEffect(() => {
    stores.observe(device);
    setup.observe(device);
  }, [stores, setup, device]);
  useEffect(() => {
    if (offered) setup.offerFirmware();
  }, [setup, offered]);
  useEffect(() => {
    const known = job ? jobsStore.getState().jobs[job] : undefined;
    if (known) stores.firmware.attach(known);
  }, [stores, job]);
  return { setup, firmware: stores.firmware, offered };
}

function Pager({ setup }: { setup: SetupStore }) {
  const flow = useStore(setup, (s) => s.flow);
  const { n, total } = stepCount(flow);
  return (
    <div className="onboarding-pager">
      <span className="onboarding-dots" aria-hidden>
        {flow.steps.map((step, i) => (
          <i key={step} className={clsx(i + 1 === n && 'is-current', i + 1 < n && 'is-past')} />
        ))}
      </span>
      <span className="onboarding-count tabular" data-testid="onboarding-count">
        Step {n} of {total}
      </span>
    </div>
  );
}

function Choice({ name, value, checked, onChoose, title, hint, icon, glyph, testId }: {
  name: string;
  value: string;
  checked: boolean;
  onChoose(): void;
  title: string;
  hint?: string;
  icon?: LucideIcon;
  glyph?: ReactNode;
  testId: string;
}) {
  const Icon = icon;
  return (
    <label className={clsx('choice', checked && 'is-chosen')} data-testid={testId} data-checked={checked || undefined}>
      <input className="choice-input" type="radio" name={name} value={value} checked={checked} onChange={onChoose} data-autofocus={checked || undefined} />
      {(Icon || glyph) && (
        <span className="choice-glyph" aria-hidden>
          {Icon ? <Icon size={18} strokeWidth={1.9} /> : glyph}
        </span>
      )}
      <span className="choice-text">
        <span className="choice-title">{title}</span>
        {hint && <span className="choice-hint">{hint}</span>}
      </span>
    </label>
  );
}

function NameStep({ setup, device, headingId }: { setup: SetupStore; device: DeviceSummary; headingId: string }) {
  const id = useId();
  const name = useStore(setup, (s) => s.name);
  const problem = useStore(setup, (s) => s.nameError);
  const needs = useStore(setup, (s) => s.needsFirmware);
  const hint = needs ? needsFirmwareField({ device: device.name ?? 'the Device', since: null }) : (problem ?? 'Stored on the Device, so it keeps its name on any PC.');
  return (
    <>
      <h3 className="onboarding-heading" id={headingId}>
        Name this Device
      </h3>
      <form
        className="field"
        onSubmit={(event) => {
          event.preventDefault();
          void setup.submitName();
        }}
      >
        <label className="field-label" htmlFor={id}>
          Name
        </label>
        <input
          id={id}
          className="input"
          value={name}
          placeholder="Desk"
          disabled={needs}
          spellCheck={false}
          autoComplete="off"
          aria-invalid={problem !== null}
          aria-describedby={`${id}-hint`}
          onChange={(event) => setup.setName(event.target.value)}
          data-autofocus
          data-testid="onboarding-name"
        />
        <p id={`${id}-hint`} className={clsx('field-hint', (problem || needs) && 'is-warning')} data-testid="onboarding-name-hint">
          {hint}
        </p>
      </form>
    </>
  );
}

function OrientationGlyph({ orientation }: { orientation: Orientation }) {
  return <span className={clsx('orientation-glyph', `is-${orientation}`)} />;
}

function OrientationStep({ setup, headingId }: { setup: SetupStore; headingId: string }) {
  const chosen = useStore(setup, (s) => s.orientation);
  const id = useId();
  return (
    <>
      <h3 className="onboarding-heading" id={headingId}>
        How does it stand?
      </h3>
      <div className="choices choices-two" role="radiogroup" aria-labelledby={headingId} aria-describedby={`${id}-hint`}>
        {ORIENTATIONS.map((o) => (
          <Choice
            key={o.value}
            name={`${id}-orientation`}
            value={o.value}
            checked={chosen === o.value}
            onChoose={() => setup.chooseOrientation(o.value)}
            title={o.label}
            glyph={<OrientationGlyph orientation={o.value} />}
            testId={`onboarding-${o.value}`}
          />
        ))}
      </div>
      <p className="field-hint" id={`${id}-hint`}>
        Each orientation keeps its own Layout. You can change it later in the Device panel.
      </p>
    </>
  );
}

interface LayoutOption {
  value: LayoutChoice;
  title: string;
  hint: string;
  icon: LucideIcon;
}

const STARTING_LAYOUTS: readonly LayoutOption[] = [
  { value: 'starter', title: 'Starter Layout', hint: 'Media controls and everyday shortcuts, ready to change.', icon: Sparkles },
  { value: 'blank', title: 'Blank Layout', hint: 'One empty Page with room for 12 Tiles.', icon: LayoutGrid },
];

const keepOption = (layout: Layout): LayoutOption => ({ value: 'keep', title: `Keep “${layout.name}”`, hint: describeOnDevice(layout), icon: Usb });

function LayoutStep({ setup, headingId }: { setup: SetupStore; headingId: string }) {
  const chosen = useStore(setup, (s) => s.choice);
  const onDevice = useStore(setup, (s) => s.onDevice);
  const reading = useStore(setup, (s) => s.busy === 'reading');
  const id = useId();
  const options = onDevice ? [keepOption(onDevice.layout), ...STARTING_LAYOUTS] : STARTING_LAYOUTS;
  return (
    <>
      <h3 className="onboarding-heading" id={headingId}>
        Pick a starting Layout
      </h3>
      <div className="choices" role="radiogroup" aria-labelledby={headingId} aria-busy={reading || undefined}>
        {options.map((o) => (
          <Choice
            key={o.value}
            name={`${id}-layout`}
            value={o.value}
            checked={chosen === o.value}
            onChoose={() => setup.chooseLayout(o.value)}
            title={o.title}
            hint={o.hint}
            icon={o.icon}
            testId={`onboarding-${o.value}`}
          />
        ))}
      </div>
    </>
  );
}

function FirmwareStep({ firmware, device, offered, headingId }: { firmware: FirmwareUpdateStore; device: DeviceSummary; offered: string | null; headingId: string }) {
  const view = useStore(firmware, (s) => s.view);
  const log = useStore(firmware, (s) => s.log);
  const called = device.name ?? 'the Device';
  const current = device.firmware?.version ?? '';
  const progress: CSSProperties = { transform: `scaleX(${(view.step === 'running' ? (view.pct ?? 0) : 0) / 100})` };
  return (
    <>
      <h3 className="onboarding-heading" id={headingId}>
        Update Firmware
      </h3>
      {view.step === 'idle' && (
        <p className="onboarding-text">
          {opening(called)} runs Firmware {current}. Firmware {offered ?? ''} adds newer Tiles.
        </p>
      )}
      {inProgress(view) && (
        <div className="onboarding-progress-group">
          <p className="onboarding-text tabular" role="status" data-testid="onboarding-firmware-status">
            {view.text}
          </p>
          <div className="onboarding-progress" role="progressbar" aria-labelledby={headingId} aria-valuemin={0} aria-valuemax={100} aria-valuenow={view.step === 'running' ? (view.pct ?? undefined) : undefined}>
            <span style={progress} />
          </div>
          <p className="field-hint">Keep {called} plugged in. It restarts once.</p>
        </div>
      )}
      {view.step === 'done' && (
        <p className="onboarding-text is-ok" role="status" data-testid="onboarding-firmware-status">
          Updated to {view.version}
        </p>
      )}
      {view.step === 'cancelled' && (
        <p className="onboarding-text" role="status" data-testid="onboarding-firmware-status">
          {view.text}
        </p>
      )}
      {view.step === 'failed' && (
        <div className="onboarding-failure" role="alert" data-testid="onboarding-firmware-status" data-code={view.code}>
          <p className="onboarding-text is-error">
            Couldn't update. {opening(called)} went back to {current} and still works.
          </p>
          <details className="onboarding-details">
            <summary>Details</summary>
            <p>{[view.text, ...log].join('\n')}</p>
          </details>
        </div>
      )}
    </>
  );
}

function WifiStep({ device, headingId }: { device: DeviceSummary; headingId: string }) {
  const called = device.name ?? 'the Device';
  return (
    <>
      <h3 className="onboarding-heading" id={headingId}>
        Wi-Fi and Bluetooth
      </h3>
      <div className="onboarding-note" data-testid="onboarding-wifi">
        <span className="choice-glyph" aria-hidden>
          <Wifi size={18} strokeWidth={1.9} />
        </span>
        <p className="onboarding-text">
          {opening(called)} runs the radio build. Wi-Fi and Bluetooth stay off until you set them up.
        </p>
      </div>
      <p className="field-hint">Set up Wi-Fi in the Device panel, now or later. Until then, {called} works over USB as usual.</p>
    </>
  );
}

function DoneStep({ device, headingId }: { device: DeviceSummary; headingId: string }) {
  const called = device.name ?? 'the Device';
  return (
    <>
      <h3 className="onboarding-heading onboarding-heading-done" id={headingId}>
        {opening(called)} is ready
      </h3>
      <p className="onboarding-text">Tap a Tile on {called} to try it, or edit its Layout here.</p>
    </>
  );
}

function stageLayout(state: Pick<SetupState, 'flow' | 'choice' | 'orientation' | 'onDevice'>): Layout | null {
  const { flow, choice, orientation, onDevice } = state;
  if (flow.at === 'layout') {
    if (choice === 'keep') return onDevice?.layout ?? null;
    return choice === 'starter' ? starterLayout(orientation) : newLayout(orientation);
  }
  if (flow.at === 'done' || flow.at === 'wifi' || (flow.at === 'firmware' && flow.steps.indexOf('firmware') > flow.steps.indexOf('layout'))) return onDevice?.layout ?? null;
  return null;
}

function Stage({ setup, device }: { setup: SetupStore; device: DeviceSummary }) {
  const typed = useStore(setup, (s) => s.name.trim());
  const flow = useStore(setup, (s) => s.flow);
  const choice = useStore(setup, (s) => s.choice);
  const orientation = useStore(setup, (s) => s.orientation);
  const onDevice = useStore(setup, (s) => s.onDevice);
  const shown = useMemo(() => stageLayout({ flow, choice, orientation, onDevice }), [flow, choice, orientation, onDevice]);
  const at = flow.at;
  const status = statusOf(device.link.state);
  const name = typed || device.name || 'Unnamed Device';
  return (
    <div className="onboarding-stage" aria-hidden>
      <div className={clsx('onboarding-device', `is-${shown?.orientation ?? orientation}`)} data-step={at}>
        <DevicePreview preview={shown?.pages.length ? { layout: shown, index: 0 } : null} live={status.live} orientation={orientation} />
      </div>
      <div className="onboarding-plate">
        <span className={clsx('device-name', !typed && !device.name && 'is-unnamed')} title={name}>
          {name}
        </span>
        <span className={clsx('status', status.live && 'is-live')}>
          <i className="led" />
          {status.words}
        </span>
      </div>
    </div>
  );
}

function Footer({ setup, firmware, device, onClose }: { setup: SetupStore; firmware: FirmwareUpdateStore; device: DeviceSummary; onClose(then: CloseThen): void }) {
  const at = useStore(setup, (s) => s.flow.at);
  const first = useStore(setup, (s) => s.flow.steps[0] === s.flow.at);
  const busy = useStore(setup, (s) => s.busy);
  const error = useStore(setup, (s) => s.error);
  const view = useStore(firmware, (s) => s.view);
  const live = device.link.state === 'connected';
  const updating = inProgress(view) || device.link.state === 'updating';
  const held = busy !== null || updating;
  const offline = !live && !held && at !== 'done';
  const called = device.name ?? 'the Device';

  const submit = () => {
    if (!live || held) return;
    switch (at) {
      case 'name':
        return void setup.submitName();
      case 'orientation':
        return void setup.submitOrientation();
      case 'layout':
        return void setup.submitLayout();
      case 'firmware':
      case 'wifi':
      case 'done':
        return setup.next();
      default: {
        const _exhaustive: never = at;
        return _exhaustive;
      }
    }
  };

  const back = !first && at !== 'done' && (
    <button type="button" className={button()} aria-disabled={held || undefined} onClick={() => !held && setup.back()} data-testid="onboarding-back">
      Back
    </button>
  );

  let actions: ReactNode;
  if (at === 'done') {
    actions = (
      <>
        <button key="edit" type="button" className={button()} onClick={() => onClose('edit')} data-testid="onboarding-edit">
          Edit Layout
        </button>
        <button key="done" type="button" className={button({ kind: 'primary' })} onClick={() => onClose('stay')} data-autofocus data-testid="onboarding-done">
          Done
        </button>
      </>
    );
  } else if (at === 'firmware' && view.step !== 'done') {
    const failed = view.step === 'failed';
    actions = (
      <>
        <button key="skip" type="button" className={button()} aria-disabled={updating || undefined} onClick={() => !updating && setup.next()} data-testid="onboarding-skip">
          Skip
        </button>
        <button
          key="update"
          type="button"
          className={button({ kind: 'primary' })}
          aria-disabled={!live || updating || undefined}
          onClick={() => live && !updating && void firmware.start('bundled')}
          data-autofocus
          data-testid="onboarding-update"
        >
          {updating ? 'Updating…' : failed ? 'Retry' : 'Update'}
        </button>
      </>
    );
  } else {
    actions = (
      <button
        key="next"
        type="button"
        className={button({ kind: 'primary' })}
        aria-disabled={!live || held || undefined}
        onClick={submit}
        data-autofocus={at === 'firmware' || at === 'wifi' || undefined}
        data-testid="onboarding-next"
      >
        {busy ? BUSY_LABEL[busy] : 'Next'}
      </button>
    );
  }

  return (
    <div className="onboarding-foot">
      {offline && (
        <p className="field-hint is-warning" data-testid="onboarding-offline">
          Plug in {called} to change its settings.
        </p>
      )}
      {error && (
        <div className="onboarding-failure" role="alert" data-testid="onboarding-error">
          <p className="onboarding-text is-error">{error.text}</p>
          {error.detail && (
            <details className="onboarding-details">
              <summary>Details</summary>
              <p>{error.detail}</p>
            </details>
          )}
        </div>
      )}
      <div className="onboarding-buttons">
        {back}
        <span className="onboarding-buttons-end">{actions}</span>
      </div>
    </div>
  );
}

export function Onboarding({ device, open, onClose, onClosed }: { device: DeviceSummary; open: boolean; onClose(then: CloseThen): void; onClosed(): void }) {
  const { setup, firmware, offered } = useSetup(device);
  const at = useStore(setup, (s) => s.flow.at);
  const headingId = useId();
  const body = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    if (!open) return;
    const target = body.current?.querySelector<HTMLElement>('[data-autofocus]');
    target?.focus({ preventScroll: true });
  }, [at, open]);

  return (
    <Dialog.Root
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose('stay');
      }}
      onOpenChangeComplete={(next) => {
        if (!next) onClosed();
      }}
      disablePointerDismissal
    >
      <Dialog.Portal>
        <Dialog.Backdrop className="scrim" />
        <Dialog.Popup
          className="dialog dialog-onboarding"
          initialFocus={() => body.current?.querySelector<HTMLElement>('[data-autofocus]') ?? true}
          data-testid="onboarding"
          data-step={at}
          data-device-id={device.id}
        >
          <Stage setup={setup} device={device} />
          <div className="onboarding-main" ref={body}>
            <div className="dialog-head">
              <Dialog.Title className="dialog-title">Set up your Device</Dialog.Title>
              <Dialog.Close className={button({ kind: 'icon' })} aria-label="Close" data-testid="onboarding-close">
                <X size={18} strokeWidth={1.9} aria-hidden />
              </Dialog.Close>
            </div>
            <Pager setup={setup} />
            <section className="onboarding-step" aria-labelledby={headingId}>
              {at === 'name' && <NameStep setup={setup} device={device} headingId={headingId} />}
              {at === 'orientation' && <OrientationStep setup={setup} headingId={headingId} />}
              {at === 'layout' && <LayoutStep setup={setup} headingId={headingId} />}
              {at === 'firmware' && <FirmwareStep firmware={firmware} device={device} offered={offered} headingId={headingId} />}
              {at === 'wifi' && <WifiStep device={device} headingId={headingId} />}
              {at === 'done' && <DoneStep device={device} headingId={headingId} />}
            </section>
            <Footer setup={setup} firmware={firmware} device={device} onClose={onClose} />
          </div>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
