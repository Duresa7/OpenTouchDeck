import { useEffect, useMemo } from 'react';
import clsx from 'clsx';
import { ArrowUpCircle, Cable } from 'lucide-react';
import type { DeviceSummary } from '../lib/companion/api.ts';
import { useCompanion } from '../lib/companion/connection.ts';
import { useDevices } from '../lib/companion/devices.ts';
import { useControlNotice } from '../lib/device/controls.ts';
import { button } from '../shell/button.ts';
import { useUi } from '../state/ui.ts';
import { DevicePreview } from './DevicePreview.tsx';
import { useDeviceLayout } from './home-layouts.ts';
import { CHECKING_CONTROLS, deviceWidth, firmwareUpdate, lastSeen, NEEDS_SEND, previewOf, shownOrientation, deviceStatus, uncheckedText } from './home-model.ts';
import { boardCards, boardInstallFor, boardsUi, useBoardsUi } from './new-boards.ts';
import { IgnoredBoards, NewBoardCard } from './NewBoardCard.tsx';
import './home.css';

function ControlsNotice({ device, name }: { device: DeviceSummary; name: string }) {
  const notice = useControlNotice(device);
  const companion = useCompanion();
  const openDevice = useUi((s) => s.openDevice);
  if (notice.kind === 'none') return null;
  if (notice.kind === 'checking') {
    return (
      <span className="device-controls" role="status" data-testid="controls-notice" data-state="checking">
        <span className="status"><i className="led" aria-hidden />{CHECKING_CONTROLS}</span>
      </span>
    );
  }
  if (notice.kind === 'unchecked') {
    return (
      <span className="device-controls" role="status" data-testid="controls-notice" data-state="unchecked" data-reason={notice.reason}>
        <span className="status is-waiting"><i className="led" aria-hidden />{uncheckedText(notice.reason, name)}</span>
      </span>
    );
  }
  const review = async () => {
    openDevice(device.id);
    if (device.name === null) return;
    const { editorFor, reviewLayout } = await import('./layout/editor.ts');
    reviewLayout(editorFor(companion), { id: device.id, name: device.name }, notice.orientation);
  };
  return (
    <span className="device-controls" data-testid="controls-notice" data-state={notice.reason}>
      <span className="status is-waiting"><i className="led" aria-hidden />{NEEDS_SEND}</span>
      <button type="button" className={button({ size: 'small' })} aria-label={`Review Layout for ${name}`} onClick={() => void review()} data-testid="controls-review">
        Review Layout
      </button>
    </span>
  );
}

function DeviceCard({ device, width }: { device: DeviceSummary; width: number }) {
  const openDevice = useUi((s) => s.openDevice);
  const status = deviceStatus(device);
  const name = device.name ?? 'Unnamed Device';
  const seen = !status.live && lastSeen(device);
  const layout = useDeviceLayout(device);
  return <li className="device-figure" style={{ width }}>
    <button type="button" className="device-hit" title="Edit Layout" aria-label={`Edit ${name}`} onClick={() => openDevice(device.id)}>
      <DevicePreview preview={previewOf(layout, device.layout)} live={status.live} orientation={shownOrientation(device)} empty={status.live && layout === null} />
    </button>
    <div className="device-caption">
      <span className="device-name" title={name}>{name}</span>
      <span className="device-meta">
        <span className={clsx('status', status.live && 'is-live')}><i className="led" aria-hidden />{status.words}</span>
        {seen && <span className="device-seen">{seen}</span>}
        {firmwareUpdate(device) && <span className="badge-signal"><ArrowUpCircle size={14} aria-hidden />Firmware update</span>}
      </span>
      <ControlsNotice device={device} name={name} />
    </div>
  </li>;
}

function NoDevices() {
  const openHelp = useUi((s) => s.openTroubleshooting);
  return <div className="empty">
    <div className="device-ghost"><DevicePreview preview={null} live={false} /></div>
    <h1 className="empty-title">Plug in your Device</h1>
    <p className="empty-text">Connect an Open TouchDeck to this PC with a USB data cable.</p>
    <p className="empty-text quiet">New board? Plug it in the same way and you’ll be offered a Firmware install.</p>
    <button type="button" className={button()} onClick={openHelp}><Cable size={16} strokeWidth={1.9} aria-hidden />Connection help</button>
  </div>;
}

function useHandedOff(devices: DeviceSummary[]) {
  const companion = useCompanion();
  const started = useBoardsUi((s) => s.started);
  useEffect(() => {
    for (const device of devices) {
      if (!started[device.id]) continue;
      boardsUi.getState().end(device.id);
      boardInstallFor(companion, device).dismiss();
    }
  }, [companion, devices, started]);
}

export function Home() {
  const devices = useDevices((s) => s.devices);
  const newBoards = useDevices((s) => s.newBoards);
  const live = useDevices((s) => s.status === 'live');
  const ignored = useBoardsUi((s) => s.ignored);
  const started = useBoardsUi((s) => s.started);
  const boards = useMemo(() => boardCards(newBoards, devices, { ignored, started }), [newBoards, devices, ignored, started]);
  useHandedOff(devices);
  const count = devices.length + boards.shown.length;
  const width = deviceWidth(count);
  return <main className={clsx('home', count === 1 && 'home-single')}>
    {count ? <ul className="device-row" aria-label="Devices">
      {devices.map((device) => <DeviceCard key={device.id} device={device} width={width} />)}
      {boards.shown.map((board) => <NewBoardCard key={board.id} board={board} width={width} />)}
    </ul> : live && <NoDevices />}
    <IgnoredBoards boards={boards.ignored} />
  </main>;
}
