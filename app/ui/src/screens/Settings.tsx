import { useEffect, useId, useRef, type ComponentType, type ReactNode } from 'react';
import { Toggle } from '@base-ui/react/toggle';
import { ToggleGroup } from '@base-ui/react/toggle-group';
import { CircleAlert, Copy, Globe, Info, LifeBuoy, SlidersHorizontal, Tablet, type LucideProps } from 'lucide-react';
import { toast } from 'sonner';
import type { DeviceSummary } from '../lib/companion/api.ts';
import { useDevices } from '../lib/companion/devices.ts';
import { button } from '../shell/button.ts';
import { About } from '../shell/settings/About.tsx';
import { PairedBrowsers } from '../shell/settings/PairedBrowsers.tsx';
import { StartAtSignIn } from '../shell/settings/StartAtSignIn.tsx';
import { TroubleshootingView } from '../shell/settings/Troubleshooting.tsx';
import { useErrors, errorsStore, type ErrorEntry } from '../state/errors.ts';
import type { Appearance } from '../state/theme.ts';
import { useUi, type SettingsSection } from '../state/ui.ts';
import { deviceStatus } from './home-model.ts';
import { DeviceSettings } from './layout/DeviceSettings.tsx';
import '../styles/layout.css';
import '../styles/settings.css';

const appearances: { value: Appearance; label: string }[] = [
  { value: 'system', label: 'Match Windows' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
];

function Head({ title, lede }: { title: ReactNode; lede?: ReactNode }) {
  const heading = useRef<HTMLHeadingElement>(null);
  // Moving between categories puts focus on the new title, so a screen reader hears where it landed.
  useEffect(() => heading.current?.focus({ preventScroll: true }), []);
  return (
    <header className="page-head">
      <h1 className="page-title" ref={heading} tabIndex={-1} data-testid="settings-title">
        {title}
      </h1>
      {lede && <p className="page-lede">{lede}</p>}
    </header>
  );
}

function General() {
  const appearance = useUi((s) => s.appearance);
  const setAppearance = useUi((s) => s.setAppearance);
  const id = useId();
  return (
    <>
      <Head title="General" />
      <section className="settings-section">
        <div className="group">
          <StartAtSignIn />
          <div className="row">
            <span className="row-text">
              <span className="row-title" id={`${id}-appearance`}>
                Appearance
              </span>
            </span>
            <ToggleGroup
              value={[appearance]}
              onValueChange={(value) => {
                const next = appearances.find((a) => a.value === value[0]);
                if (next) setAppearance(next.value);
              }}
              aria-labelledby={`${id}-appearance`}
              className="segmented"
            >
              {appearances.map((a) => (
                <Toggle key={a.value} value={a.value} className="segment">
                  {a.label}
                </Toggle>
              ))}
            </ToggleGroup>
          </div>
        </div>
      </section>
    </>
  );
}

function Devices({ device }: { device: DeviceSummary | null }) {
  if (!device) {
    return (
      <>
        <Head title="Devices" />
        <div className="group">
          <div className="row">
            <span className="row-note">No Device connected. Plug one in with a USB cable.</span>
          </div>
        </div>
      </>
    );
  }
  const name = device.name ?? 'Unnamed Device';
  return (
    <>
      <Head title={name} lede={deviceStatus(device).words} />
      <DeviceSettings key={device.id} device={device} />
    </>
  );
}

const TIME = new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit', second: '2-digit' });

function copyText(entry: ErrorEntry) {
  return [`${TIME.format(entry.at)} · ${entry.where}`, entry.text, entry.detail].filter(Boolean).join('\n');
}

function ErrorRow({ entry }: { entry: ErrorEntry }) {
  const copy = () =>
    void navigator.clipboard?.writeText(copyText(entry)).then(
      () => toast.success('Copied'),
      () => toast.error("Couldn't copy."),
    );
  return (
    <li className="error-entry" data-testid="error-entry" data-id={entry.id}>
      <div className="error-entry-head">
        <span className="row-text">
          <span className="row-title error-entry-text">{entry.text}</span>
          <span className="row-sub">
            <time dateTime={new Date(entry.at).toISOString()}>{TIME.format(entry.at)}</time> · {entry.where}
          </span>
        </span>
        <button type="button" className={button({ kind: 'icon' })} aria-label="Copy" onClick={copy} data-testid="error-copy">
          <Copy size={16} strokeWidth={1.9} aria-hidden />
        </button>
      </div>
      {entry.detail && (
        <pre className="error-entry-detail" data-testid="error-detail">
          {entry.detail}
        </pre>
      )}
    </li>
  );
}

function Errors() {
  const entries = useErrors((s) => s.entries);
  return (
    <>
      <Head title="Errors" lede="What went wrong since Open TouchDeck opened, newest first." />
      {entries.length ? (
        <>
          <div className="settings-section-head">
            <span className="row-sub tabular">{entries.length === 1 ? '1 error' : `${entries.length} errors`}</span>
            <button type="button" className={button({ size: 'small' })} onClick={() => errorsStore.getState().clear()} data-testid="errors-clear">
              Clear
            </button>
          </div>
          <ul className="error-list" data-testid="error-list">
            {entries.map((entry) => (
              <ErrorRow key={entry.id} entry={entry} />
            ))}
          </ul>
        </>
      ) : (
        <div className="group" data-testid="errors-empty">
          <div className="row">
            <span className="row-note">No errors.</span>
          </div>
        </div>
      )}
    </>
  );
}

function Content({ section, device }: { section: SettingsSection; device: DeviceSummary | null }) {
  switch (section) {
    case 'general':
      return <General />;
    case 'device':
      return <Devices device={device} />;
    case 'browsers':
      return (
        <>
          <Head title="Paired browsers" lede="Browsers that can open Open TouchDeck. Forget one to sign it out." />
          <PairedBrowsers titled={false} />
        </>
      );
    case 'troubleshooting':
      return (
        <>
          <Head title="Troubleshooting" lede="Device details, and tools to check that taps reach this PC." />
          <TroubleshootingView />
        </>
      );
    case 'errors':
      return <Errors />;
    case 'about':
      return (
        <>
          <Head title="About" />
          <div className="group">
            <About />
          </div>
        </>
      );
    default: {
      const unknown: never = section;
      return unknown;
    }
  }
}

function NavItem({
  label,
  icon: Icon,
  current,
  count,
  onClick,
  testId,
  device,
}: {
  label: string;
  icon: ComponentType<LucideProps>;
  current: boolean;
  count?: number;
  onClick(): void;
  testId: string;
  device?: string;
}) {
  return (
    <li>
      <button type="button" className="settings-nav-item" aria-current={current ? 'page' : undefined} onClick={onClick} data-testid={testId} data-device={device}>
        <Icon size={16} strokeWidth={1.9} aria-hidden />
        {/* A Device's name can run past the sidebar, so the whole name waits in a tooltip. */}
        <span className="settings-nav-name" title={device ? label : undefined}>
          {label}
        </span>
        {count ? (
          <span className="settings-nav-count tabular" aria-label={count === 1 ? '1 error' : `${count} errors`}>
            {count}
          </span>
        ) : null}
      </button>
    </li>
  );
}

/**
 * Settings: a full page with its categories down the left. Each Device has its own entry under Devices. The category
 * shown lives in the UI store, so other screens can open one, such as Errors from an error card.
 */
export function Settings() {
  const section = useUi((s) => s.settingsSection);
  const wanted = useUi((s) => s.settingsDevice);
  const editing = useUi((s) => s.deviceId);
  const open = useUi((s) => s.openSettings);
  const devices = useDevices((s) => s.devices);
  const errors = useErrors((s) => s.entries.length);
  const device = devices.find((d) => d.id === wanted) ?? devices.find((d) => d.id === editing) ?? devices[0] ?? null;

  return (
    <main className="settings-screen" data-testid="settings" data-section={section}>
      <nav className="settings-nav" aria-label="Settings">
        <p className="settings-nav-title">Settings</p>
        <ul className="settings-nav-list">
          <NavItem label="General" icon={SlidersHorizontal} current={section === 'general'} onClick={() => open('general')} testId="settings-nav-general" />
        </ul>
        <p className="settings-nav-label">Devices</p>
        <ul className="settings-nav-list">
          {devices.length ? (
            devices.map((d) => (
              <NavItem
                key={d.id}
                label={d.name ?? 'Unnamed Device'}
                icon={Tablet}
                current={section === 'device' && device?.id === d.id}
                onClick={() => open('device', d.id)}
                testId="settings-nav-device"
                device={d.id}
              />
            ))
          ) : (
            <NavItem label="No Device" icon={Tablet} current={section === 'device'} onClick={() => open('device')} testId="settings-nav-device" />
          )}
        </ul>
        <p className="settings-nav-label">App</p>
        <ul className="settings-nav-list">
          <NavItem label="Paired browsers" icon={Globe} current={section === 'browsers'} onClick={() => open('browsers')} testId="settings-nav-browsers" />
          <NavItem label="Troubleshooting" icon={LifeBuoy} current={section === 'troubleshooting'} onClick={() => open('troubleshooting')} testId="settings-nav-troubleshooting" />
          <NavItem label="Errors" icon={CircleAlert} current={section === 'errors'} count={errors} onClick={() => open('errors')} testId="settings-nav-errors" />
          <NavItem label="About" icon={Info} current={section === 'about'} onClick={() => open('about')} testId="settings-nav-about" />
        </ul>
      </nav>
      <div className="settings-content" key={section === 'device' ? `device:${device?.id ?? ''}` : section}>
        <Content section={section} device={device} />
      </div>
    </main>
  );
}
