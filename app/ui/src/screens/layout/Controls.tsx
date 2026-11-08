import { useId, useMemo } from 'react';
import { useStore } from 'zustand';
import { useDevices } from '../../lib/companion/devices.ts';
import { useIntegrations } from '../../lib/companion/integrations.ts';
import { controlsStore, noticeOf, type ControlNotice, type NeedsSend } from '../../lib/device/controls.ts';
import { controlBindings, controlWords, type ControlBinding } from '../../lib/layout/controls.ts';
import { plural } from '../../lib/layout/messages.ts';
import type { EditorStore } from '../../lib/editor/store.ts';
import type { Layout, Orientation } from '../../lib/layout/model.ts';
import { button } from '../../shell/button.ts';
import { CHECKING_CONTROLS, NEEDS_SEND, uncheckedText } from '../home-model.ts';
import { reviewLayout, useEditorState } from './editor.ts';

const NAMES: Record<Orientation, string> = { landscape: 'Landscape', portrait: 'Portrait' };

export function useEditorControlNotice(store: EditorStore): ControlNotice {
  const id = useEditorState(store, (s) => s.device?.id);
  const device = useDevices((s) => s.devices.find((d) => d.id === id));
  const watch = useStore(controlsStore, (s) => (id ? s.watched[id] : undefined));
  return device ? noticeOf(watch, device) : { kind: 'none' };
}

function needsText(reason: NeedsSend, device: string) {
  switch (reason) {
    case 'no-pushed-copy':
      return "These Tiles show live values, but their controls aren't set up on this PC.";
    case 'differs':
      return `The Layout on ${device} differs from the one this PC sent.`;
    default: {
      const exhaustive: never = reason;
      return exhaustive;
    }
  }
}

export function ControlNoticeBar({ store, device }: { store: EditorStore; device: string }) {
  const notice = useEditorControlNotice(store);
  const shown = useEditorState(store, (s) => s.orientation);
  const busy = useEditorState(store, (s) => s.busy);
  const editing = useEditorState(store, (s) => s.device);
  const titleId = useId();
  if (notice.kind === 'none') return null;
  if (notice.kind === 'checking') {
    return (
      <section className="controls-notice is-checking" role="status" data-testid="controls-notice" data-state="checking">
        <p className="controls-notice-title">
          <i className="status-box" data-tone="wait" aria-hidden />
          {CHECKING_CONTROLS}
        </p>
      </section>
    );
  }
  if (notice.kind === 'unchecked') {
    return (
      <section className="controls-notice is-unchecked" role="status" data-testid="controls-notice" data-state="unchecked" data-reason={notice.reason}>
        <p className="controls-notice-title">
          <i className="status-box" data-tone="wait" aria-hidden />
          {uncheckedText(notice.reason, device)}
        </p>
      </section>
    );
  }
  const other = notice.orientation !== shown;
  const action = other ? (
    <button type="button" className={button({ size: 'small' })} onClick={() => store.setOrientation(notice.orientation)} data-testid="controls-switch">
      Switch to {NAMES[notice.orientation]}
    </button>
  ) : (
    <button
      type="button"
      className={button({ size: 'small' })}
      aria-disabled={busy !== null}
      onClick={() => editing && busy === null && reviewLayout(store, editing, notice.orientation)}
      data-testid="controls-review"
    >
      {busy === 'load' ? 'Loading…' : 'Review Layout'}
    </button>
  );
  return (
    <section className="controls-notice" aria-labelledby={titleId} data-testid="controls-notice" data-state={other ? 'other-orientation' : notice.reason}>
      <div className="controls-notice-head">
        <h2 className="controls-notice-title" id={titleId}>
          <i className="status-box" data-tone="wait" aria-hidden />
          {NEEDS_SEND}
        </h2>
        {action}
      </div>
      <p className="controls-notice-text">
        {other ? (
          <span data-testid="controls-notice-body">
            {device} shows its {notice.orientation} Layout. Switch to {NAMES[notice.orientation]} to review it.
          </span>
        ) : (
          <>
            <span data-testid="controls-notice-body">{needsText(notice.reason, device)}</span>{' '}
            <span className="controls-notice-note">Sending replaces the Layout on {device} and lets its Tiles control the Integrations shown here.</span>
          </>
        )}
      </p>
    </section>
  );
}

function Named({ binding }: { binding: ControlBinding }) {
  const { integration, account } = binding;
  return (
    <span className="control-row-line">
      {integration.kind === 'known' ? (
        <span data-testid="control-integration">{integration.name}</span>
      ) : (
        <span className="is-unknown" data-testid="control-integration">
          Unknown Integration “{integration.id}”
        </span>
      )}
      <span aria-hidden> · </span>
      {account.kind === 'default' ? (
        <span data-testid="control-account">Default account</span>
      ) : (
        <span className="is-unknown" data-testid="control-account">
          Unknown account “{account.id}”
        </span>
      )}
    </span>
  );
}

export function ControlReview({ store, layout }: { store: EditorStore; layout: Layout }) {
  const notice = useEditorControlNotice(store);
  const shown = useEditorState(store, (s) => s.orientation);
  const integrations = useIntegrations((s) => s.integrations);
  const bindings = useMemo(() => controlBindings(layout, integrations), [layout, integrations]);
  const titleId = useId();
  if (notice.kind === 'none' || notice.orientation !== shown) return null;
  const open = (binding: ControlBinding) => {
    store.setMode('edit');
    store.selectPage(binding.page.id);
    store.select(binding.tile.id);
  };
  return (
    <section className="inspector-section" aria-labelledby={titleId} data-testid="control-review">
      <div className="section-head">
        <h2 className="section-title" id={titleId}>
          Controls
        </h2>
        <span className="section-sub tabular">{plural(bindings.length, 'Tile')}</span>
      </div>
      {bindings.length ? (
        <>
          <p className="field-hint">Tiles on every Page that control an Integration. Select one to change it.</p>
          <ul className="control-list">
            {bindings.map((binding) => (
              <li key={`${binding.page.id}/${binding.tile.id}`}>
                <button type="button" className="control-row" onClick={() => open(binding)} data-testid="control-binding" data-page={binding.page.id} data-tile={binding.tile.id}>
                  <span className="control-row-head">
                    <span className="control-row-tile">{binding.tile.label}</span>
                    <span className="control-row-page">{binding.page.title} Page</span>
                  </span>
                  <Named binding={binding} />
                  <span className="control-row-parts" data-testid="control-parts">
                    {controlWords(binding.controls)}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </>
      ) : (
        <p className="field-hint">No Tile on this Layout controls an Integration.</p>
      )}
    </section>
  );
}
