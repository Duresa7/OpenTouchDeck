import { useEffect, useId, useRef, useState, type CSSProperties, type KeyboardEvent } from 'react';
import clsx from 'clsx';
import { useStore } from 'zustand';
import { Popover } from '@base-ui/react/popover';
import { Slider } from '@base-ui/react/slider';
import { Switch } from '@base-ui/react/switch';
import { Toggle } from '@base-ui/react/toggle';
import { ToggleGroup } from '@base-ui/react/toggle-group';
import { ChevronDown } from 'lucide-react';
import type { DeviceSummary, Orientation } from '../../lib/companion/api.ts';
import { useFolderPicker } from '../../lib/companion/connection.ts';
import { inProgress, type FirmwareUpdateStore } from '../../lib/device/firmware.ts';
import { needsFirmwareField } from '../../lib/device/needs-firmware.ts';
import { durationLabel, startAfterChoices } from '../../lib/device/photo-frame.ts';
import { buildChoice } from '../../lib/device/radio-build.ts';
import type { DeviceSettingsStore } from '../../lib/device/settings.ts';
import { button } from '../../shell/button.ts';
import { useBundledBuild, useDeviceStores, useOfferedFirmware } from './device.ts';
import { Select } from './fields.tsx';

const ORIENTATIONS: { value: Orientation; label: string }[] = [
  { value: 'landscape', label: 'Landscape' },
  { value: 'portrait', label: 'Portrait' },
];

const isOrientation = (value: unknown): value is Orientation => value === 'landscape' || value === 'portrait';

interface PanelProps {
  device: DeviceSummary;
  name: string;
  settings: DeviceSettingsStore;
  live: boolean;
  editable: boolean;
}

function NameField({ device, name, settings, live, editable }: PanelProps) {
  const id = useId();
  const shown = useStore(settings, (s) => s.shown?.name ?? null);
  const lacks = useStore(settings, (s) => s.needs.name);
  const needs = lacks && needsFirmwareField({ device: name, since: lacks.since });
  const problem = useStore(settings, (s) => s.problem.name);
  const value = shown ?? device.name ?? '';
  const [draft, setDraft] = useState(value);
  const dropped = useRef(false);
  useEffect(() => setDraft(value), [value]);
  const commit = () => {
    if (dropped.current) dropped.current = false;
    else if (draft !== value) void settings.rename(draft);
  };
  const keys = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') commit();
    if (event.key === 'Escape') {
      dropped.current = true;
      setDraft(value);
    }
  };
  const hint = problem ?? needs ?? `Stored on ${name}.`;
  const save = editable ? commit : () => {};
  return (
    <div className="field">
      <label className="field-label" htmlFor={id}>
        Name
      </label>
      <input
        id={id}
        className="input"
        value={draft}
        disabled={!live || needs !== undefined}
        readOnly={!editable}
        spellCheck={false}
        autoComplete="off"
        aria-invalid={problem !== undefined}
        aria-describedby={`${id}-hint`}
        onChange={(event) => setDraft(event.target.value)}
        onFocus={() => (dropped.current = false)}
        onBlur={save}
        onKeyDown={editable ? keys : undefined}
        data-testid="device-panel-name"
      />
      <p id={`${id}-hint`} className={clsx('field-hint', (problem ?? needs) && 'is-warning')} data-testid="device-panel-name-hint">
        {hint}
      </p>
    </div>
  );
}

function OrientationField({ name, settings, editable }: PanelProps) {
  const id = useId();
  const shown = useStore(settings, (s) => s.shown?.orientation);
  const confirm = useStore(settings, (s) => s.confirm);
  const current = confirm ?? (isOrientation(shown) ? shown : null);
  const label = ORIENTATIONS.find((o) => o.value === confirm)?.label ?? '';
  return (
    <div className="field">
      <span className="field-label" id={id}>
        Orientation
      </span>
      <ToggleGroup
        value={current ? [current] : []}
        onValueChange={(value) => {
          const [next] = value;
          if (isOrientation(next)) settings.chooseOrientation(next);
        }}
        disabled={!editable}
        aria-labelledby={id}
        className="segmented segmented-fill"
      >
        {ORIENTATIONS.map((o) => (
          <Toggle key={o.value} value={o.value} className="segment" data-testid={`device-panel-${o.value}`}>
            {o.label}
          </Toggle>
        ))}
      </ToggleGroup>
      {confirm && (
        <div className="panel-confirm" role="group" aria-labelledby={`${id}-confirm`} data-testid="device-panel-confirm">
          <p className="panel-confirm-title" id={`${id}-confirm`}>
            Restart {name} to switch to {label}?
          </p>
          <p className="field-hint">Each orientation keeps its own Layout.</p>
          <div className="panel-confirm-buttons">
            <button type="button" className={button({ size: 'small' })} onClick={() => settings.cancelOrientation()} data-testid="device-panel-confirm-cancel">
              Cancel
            </button>
            <button type="button" className={button({ kind: 'primary', size: 'small' })} onClick={() => void settings.confirmOrientation()} data-testid="device-panel-confirm-restart">
              Restart
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

function BrightnessField({ name, settings, editable }: PanelProps) {
  const id = useId();
  const value = useStore(settings, (s) => s.shown?.brightness ?? null);
  const lacks = useStore(settings, (s) => s.needs.brightness);
  const needs = lacks && needsFirmwareField({ device: name, since: lacks.since });
  return (
    <div className="field">
      <span className="field-label field-label-row">
        <span id={id}>Brightness</span>
        {value !== null && !needs && (
          <span className="field-value tabular" data-testid="device-panel-brightness-value">
            {value}%
          </span>
        )}
      </span>
      {needs ? (
        <p className="field-hint is-warning" data-testid="device-panel-brightness-hint">
          {needs}
        </p>
      ) : (
        <Slider.Root
          className="slider"
          value={value ?? 0}
          min={0}
          max={100}
          disabled={!editable || value === null}
          onValueChange={(next) => settings.slideBrightness(next)}
          onValueCommitted={(next) => settings.commitBrightness(next)}
          aria-labelledby={id}
          data-testid="device-panel-brightness"
        >
          <Slider.Control className="slider-control">
            <Slider.Track className="slider-track">
              <Slider.Indicator className="slider-indicator" />
              <Slider.Thumb className="slider-thumb" aria-labelledby={id} aria-valuetext={value === null ? undefined : `${value}%`} />
            </Slider.Track>
          </Slider.Control>
        </Slider.Root>
      )}
    </div>
  );
}

function PhotoFrameField({ name, settings, editable }: PanelProps) {
  const id = useId();
  const pick = useFolderPicker();
  const frame = useStore(settings, (s) => s.shown?.photo_frame ?? null);
  const lacks = useStore(settings, (s) => s.needs.photo_frame);
  const needs = lacks && needsFirmwareField({ device: name, since: lacks.since });
  const problem = useStore(settings, (s) => s.problem.folder);
  const choose = async () => {
    const folder = pick ? await pick() : null;
    if (folder) await settings.setPhotoFrame({ folder });
  };
  if (needs) {
    return (
      <div className="field">
        <span className="field-label">Photo frame</span>
        <p className="field-hint is-warning" data-testid="device-panel-photo-frame-hint">
          {needs}
        </p>
      </div>
    );
  }
  return (
    <>
      <div className="field">
        <div className="field-label field-label-row switch-row">
          <span id={`${id}-title`}>Photo frame</span>
          <Switch.Root
            className="switch"
            checked={frame?.enabled ?? false}
            disabled={!editable || frame === null}
            onCheckedChange={(enabled) => void settings.setPhotoFrame({ enabled })}
            aria-labelledby={`${id}-title`}
            aria-describedby={`${id}-hint`}
            data-testid="device-panel-photo-frame"
          >
            <Switch.Thumb className="switch-thumb" />
          </Switch.Root>
        </div>
        <p className="field-hint" id={`${id}-hint`} data-testid="device-panel-photo-frame-hint">
          Pictures from this PC when {name} sits idle, or a clock when Open TouchDeck isn't running.
        </p>
      </div>
      {frame?.enabled && (
        <>
          <div className="field">
            <label className="field-label" htmlFor={`${id}-after`}>
              Start after
            </label>
            <Select
              id={`${id}-after`}
              value={frame.timeout_s}
              disabled={!editable}
              onChange={(event) => void settings.setPhotoFrame({ timeout_s: Number(event.target.value) })}
              data-testid="device-panel-photo-frame-after"
            >
              {startAfterChoices(frame.timeout_s).map((seconds) => (
                <option key={seconds} value={seconds}>
                  {durationLabel(seconds)}
                </option>
              ))}
            </Select>
          </div>
          <div className="field">
            <span className="field-label" id={`${id}-folder`}>
              Folder
            </span>
            <div className="panel-row">
              <span
                className={clsx('panel-folder', !frame.folder && 'is-empty')}
                title={frame.folder ?? undefined}
                aria-labelledby={`${id}-folder`}
                data-testid="device-panel-folder"
              >
                {frame.folder ?? 'No folder chosen'}
              </span>
              {pick && (
                <button
                  type="button"
                  className={button({ size: 'small' })}
                  aria-disabled={!editable}
                  onClick={() => editable && void choose()}
                  data-testid="device-panel-folder-choose"
                >
                  Choose folder
                </button>
              )}
            </div>
            <p className={problem ? 'field-error' : 'field-hint'} role={problem ? 'alert' : undefined} data-testid="device-panel-folder-hint">
              {problem ?? (pick ? `Nothing is stored on ${name}.` : 'Choose a folder in the Open TouchDeck app.')}
            </p>
          </div>
        </>
      )}
    </>
  );
}

function FirmwareRow({ device, name, settings, firmware }: PanelProps & { firmware: FirmwareUpdateStore }) {
  const offered = useOfferedFirmware(device);
  const view = useStore(firmware, (s) => s.view);
  const log = useStore(firmware, (s) => s.log);
  const restart = useStore(settings, (s) => s.restart);
  const version = device.firmware?.version ?? null;
  const live = device.link.state === 'connected';
  const busy = inProgress(view);
  const updating = busy || device.link.state === 'updating';
  const status =
    busy || view.step === 'done' ? view.text : offered ? `${offered} available` : version ? 'Up to date' : 'Not reported';
  const offer = offered !== null && !busy && view.step !== 'done';
  const progress: CSSProperties = { transform: `scaleX(${(view.step === 'running' ? (view.pct ?? 0) : 100) / 100})` };
  return (
    <div className="panel-firmware" data-testid="device-panel-firmware" data-step={view.step}>
      <div className="panel-row">
        <span className="row-text">
          <span className="row-title">{version ? `Firmware ${version}` : 'Firmware'}</span>
          <span className={clsx('row-sub', view.step === 'running' && 'tabular', view.step === 'done' && 'is-ok')} data-testid="device-panel-firmware-status">
            {status}
          </span>
        </span>
        <span className="panel-row-buttons">
          {offer && (
            <button
              type="button"
              className={button({ kind: 'primary' })}
              aria-disabled={!live}
              onClick={() => live && void firmware.start('bundled')}
              data-testid="device-panel-update"
            >
              Update
            </button>
          )}
          <button
            type="button"
            className={button()}
            aria-disabled={!live || updating || restart === 'restarting'}
            onClick={() => live && !updating && restart !== 'restarting' && void settings.restart()}
            data-testid="device-panel-restart"
          >
            Restart
          </button>
        </span>
      </div>
      {view.step === 'running' && (
        <div className="firmware-progress" role="progressbar" aria-label={`Updating ${name}`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={view.pct ?? undefined}>
          <span className="firmware-progress-fill" style={progress} />
        </div>
      )}
      {view.step === 'running' && <p className="field-hint">Keep {name} plugged in. Takes about a minute. {name} restarts once.</p>}
      {view.step === 'failed' && (
        <div className="panel-status is-error" role="status" data-testid="device-panel-firmware-error" data-code={view.code}>
          <p className="panel-status-text">{view.text}</p>
          <details className="panel-details">
            <summary>Details</summary>
            <p>{log.join('\n')}</p>
          </details>
        </div>
      )}
      {view.step === 'cancelled' && (
        <div className="panel-status" role="status" data-testid="device-panel-firmware-stopped">
          <p className="panel-status-text">{view.text}</p>
          <details className="panel-details">
            <summary>Details</summary>
            <p>{log.join('\n')}</p>
          </details>
        </div>
      )}
    </div>
  );
}

function BuildField({ device, name, firmware }: PanelProps & { firmware: FirmwareUpdateStore }) {
  const id = useId();
  const bundled = useBundledBuild();
  const view = useStore(firmware, (s) => s.view);
  const choice = buildChoice(device, bundled, name);
  const [confirming, setConfirming] = useState(false);
  const live = device.link.state === 'connected';
  const busy = inProgress(view) || device.link.state === 'updating';
  const toggle = useRef<HTMLButtonElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const back = useRef(false);
  useEffect(() => setConfirming(false), [choice.current, live]);
  // The switch gives way to its confirm, so focus moves to Cancel and, after Cancel, back to the switch.
  useEffect(() => {
    if (confirming) cancel.current?.focus();
    else if (back.current) toggle.current?.focus();
    back.current = false;
  }, [confirming]);
  if (!device.firmware) return null;
  const ready = live && !busy;
  return (
    <div className="field" data-testid="device-panel-build" data-channel={choice.current}>
      <div className="panel-row">
        <span className="row-text">
          <span className="row-title" id={`${id}-title`}>
            Firmware build
          </span>
          <span className="row-sub" data-testid="device-panel-build-current">
            {choice.title}
          </span>
        </span>
        {choice.action && !confirming && (
          <span className="panel-row-buttons">
            <button
              ref={toggle}
              type="button"
              className={button({ size: 'small' })}
              aria-disabled={!ready}
              aria-describedby={`${id}-hint`}
              onClick={() => ready && setConfirming(true)}
              data-testid="device-panel-build-switch"
            >
              {choice.action}
            </button>
          </span>
        )}
      </div>
      <p className="field-hint" id={`${id}-hint`} data-testid="device-panel-build-hint">
        {choice.missing ?? choice.note}
      </p>
      {confirming && (
        <div className="panel-confirm" role="group" aria-labelledby={`${id}-confirm`} data-testid="device-panel-build-confirm">
          <p className="panel-confirm-title" id={`${id}-confirm`}>
            {choice.confirmTitle}
          </p>
          <p className={clsx('field-hint', choice.warn && 'is-warning')} data-testid="device-panel-build-note">
            {choice.confirmNote}
          </p>
          <div className="panel-confirm-buttons">
            <button
              ref={cancel}
              type="button"
              className={button({ size: 'small' })}
              onClick={() => {
                back.current = true;
                setConfirming(false);
              }}
              data-testid="device-panel-build-cancel"
            >
              Cancel
            </button>
            <button
              type="button"
              className={button({ kind: 'primary', size: 'small' })}
              aria-disabled={!ready}
              onClick={() => {
                if (!ready) return;
                setConfirming(false);
                void firmware.start('bundled', choice.target);
              }}
              data-testid="device-panel-build-confirm-switch"
            >
              {choice.warn ? 'Switch' : 'Install'}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

function Notice({ settings }: { settings: DeviceSettingsStore }) {
  const notice = useStore(settings, (s) => s.notice);
  return (
    <div className={clsx('panel-status', notice && `is-${notice.kind}`)} role="status" data-testid="device-panel-status" data-event={notice?.event} data-kind={notice?.kind}>
      {notice && (
        <>
          <p className="panel-status-text">{notice.text}</p>
          {notice.kind === 'error' && notice.detail && (
            <details className="panel-details">
              <summary>Details</summary>
              <p>{notice.detail}</p>
            </details>
          )}
        </>
      )}
    </div>
  );
}

function PanelBody({ device }: { device: DeviceSummary }) {
  const { settings, firmware } = useDeviceStores(device);
  const name = device.name ?? 'Unnamed Device';
  const live = device.link.state === 'connected';
  const loaded = useStore(settings, (s) => s.load === 'ready');
  const restarting = useStore(settings, (s) => s.restart === 'restarting');

  useEffect(() => {
    if (live) void settings.load();
  }, [settings, live]);

  const props: PanelProps = { device, name, settings, live, editable: live && loaded && !restarting };
  return (
    <>
      {!live && device.link.state !== 'updating' && !restarting && (
        <p className="field-hint" data-testid="device-panel-offline">
          Plug in {name} to change its settings.
        </p>
      )}
      <NameField {...props} />
      <OrientationField {...props} />
      <BrightnessField {...props} />
      <PhotoFrameField {...props} />
      <FirmwareRow {...props} firmware={firmware} />
      <BuildField {...props} firmware={firmware} />
      <Notice settings={settings} />
    </>
  );
}

export function DevicePanel({ device }: { device: DeviceSummary }) {
  const name = device.name ?? 'Unnamed Device';
  const popup = useRef<HTMLDivElement>(null);
  return (
    <Popover.Root>
      <Popover.Trigger className="device-switch" data-testid="device-name">
        <span className="device-switch-name" title={name}>{name}</span>
        <ChevronDown className="chev" size={16} strokeWidth={2} aria-hidden />
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Positioner className="popover-positioner" side="bottom" align="start" sideOffset={8}>
          <Popover.Popup
            ref={popup}
            className="popover device-panel"
            initialFocus={(type) => (type === 'keyboard' ? true : popup.current)}
            data-testid="device-panel"
          >
            <Popover.Title className="popover-title">{name}</Popover.Title>
            <PanelBody device={device} />
          </Popover.Popup>
        </Popover.Positioner>
      </Popover.Portal>
    </Popover.Root>
  );
}
