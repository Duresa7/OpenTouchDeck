import { useEffect, useId, useRef, useState, type MouseEvent } from 'react';
import { AlertDialog } from '@base-ui/react/alert-dialog';
import { toast } from 'sonner';
import { useStore } from 'zustand';
import type { SessionInfo } from '../../lib/companion/api.ts';
import { useCompanion } from '../../lib/companion/connection.ts';
import { button } from '../button.ts';
import { createBrowsersStore, type BrowsersStore } from './browsers.ts';

type Ask = { kind: 'forget'; browser: SessionInfo } | { kind: 'forget-all' } | { kind: 'reset' };

function wording(ask: Ask) {
  switch (ask.kind) {
    case 'forget':
      return { title: 'Forget this browser?', body: 'It needs a new code to use Open TouchDeck again.', action: 'Forget' };
    case 'forget-all':
      return { title: 'Forget all browsers?', body: 'Each needs a new code to use Open TouchDeck again.', action: 'Forget all' };
    case 'reset':
      return {
        title: 'Reset pairing?',
        body: 'Every window and browser is signed out. Browsers need a new code to use Open TouchDeck again.',
        action: 'Reset',
      };
    default: {
      const _exhaustive: never = ask;
      return _exhaustive;
    }
  }
}

function confirm(store: BrowsersStore, ask: Ask) {
  const state = store.getState();
  switch (ask.kind) {
    case 'forget':
      return state.forget(ask.browser);
    case 'forget-all':
      return state.forgetAll();
    case 'reset':
      return state.resetPairing();
    default: {
      const _exhaustive: never = ask;
      return _exhaustive;
    }
  }
}

const day = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' });
const dayTime = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });

/** Only the desktop window can open a browser on this PC: the Companion shows the browser a pairing code. */
function OpenInBrowser() {
  const companion = useCompanion();
  if (!window.openTouchDeck) return null;
  return (
    <div className="row" data-testid="open-browser-row">
      <span className="row-text">
        <span className="row-title">Open in a browser</span>
        <span className="row-sub">Pairs a browser on this PC.</span>
      </span>
      <button
        type="button"
        className={button()}
        onClick={() => void companion.auth.pair_code().catch(() => toast.error("Can't reach the Companion. Check that Open TouchDeck is running in the tray, then retry."))}
        data-testid="open-browser"
      >
        Open
      </button>
    </div>
  );
}

function BrowserRow({ browser, onForget }: { browser: SessionInfo; onForget(event: MouseEvent<HTMLButtonElement>): void }) {
  return (
    <div className="row" data-testid="paired-browser">
      <span className="row-text">
        <span className="row-title">
          {browser.label} · paired {day.format(browser.created_ms)}
        </span>
        <span className="row-sub tabular">
          Last used {dayTime.format(browser.last_used_ms)}
          {browser.current && ' · This browser'}
        </span>
      </span>
      <button type="button" className={button()} aria-label={`Forget ${browser.label}`} onClick={onForget}>
        Forget
      </button>
    </div>
  );
}

/** The browsers that can open Open TouchDeck. On its own Settings page the page title names it, so `titled` hides the heading. */
export function PairedBrowsers({ titled = true }: { titled?: boolean }) {
  const companion = useCompanion();
  const [store] = useState(() => createBrowsersStore(companion));
  const view = useStore(store, (s) => s.view);
  const problem = useStore(store, (s) => s.problem);
  const [ask, setAsk] = useState<Ask | null>(null);
  const shown = useRef<Ask>({ kind: 'reset' });
  const trigger = useRef<HTMLButtonElement | null>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const id = useId();
  useEffect(() => void store.getState().load(), [store]);

  if (ask) shown.current = ask;
  const words = wording(shown.current);
  const browsers = view.kind === 'ready' ? view.browsers : [];
  const open = (next: Ask) => (event: MouseEvent<HTMLButtonElement>) => {
    trigger.current = event.currentTarget;
    setAsk(next);
  };

  return (
    <section className="settings-section" aria-labelledby={id}>
      <div className="settings-section-head">
        <h3 id={id} ref={heading} tabIndex={-1} className={titled ? undefined : 'visually-hidden'}>
          Paired browsers
        </h3>
        {browsers.length > 1 && (
          <button type="button" className={button({ size: 'small' })} onClick={open({ kind: 'forget-all' })}>
            Forget all
          </button>
        )}
      </div>
      <div className="group" aria-busy={view.kind === 'loading' || undefined}>
        {view.kind === 'ready' && browsers.length === 0 && (
          <div className="row">
            <span className="row-note">No browsers paired yet.</span>
          </div>
        )}
        <OpenInBrowser />
        {browsers.map((browser) => (
          <BrowserRow key={browser.id} browser={browser} onForget={open({ kind: 'forget', browser })} />
        ))}
        <div className="row">
          <span className="row-text">
            <span className="row-title">Reset pairing</span>
            <span className="row-sub">Signs out every window and browser.</span>
          </span>
          <button type="button" className={button()} onClick={open({ kind: 'reset' })}>
            Reset
          </button>
        </div>
      </div>
      {problem && (
        <p className="settings-problem" role="alert">
          {problem}
        </p>
      )}

      <AlertDialog.Root open={ask !== null} onOpenChange={(next) => !next && setAsk(null)}>
        <AlertDialog.Portal>
          <AlertDialog.Backdrop className="scrim" forceRender />
          <AlertDialog.Popup className="dialog dialog-confirm" finalFocus={() => (trigger.current?.isConnected ? trigger.current : heading.current)}>
            <AlertDialog.Title className="dialog-title">{words.title}</AlertDialog.Title>
            <AlertDialog.Description className="dialog-text">{words.body}</AlertDialog.Description>
            <div className="dialog-actions">
              <AlertDialog.Close className={button()}>Cancel</AlertDialog.Close>
              <button
                type="button"
                className={button({ kind: 'primary' })}
                onClick={() => {
                  if (ask) void confirm(store, ask);
                  setAsk(null);
                }}
              >
                {words.action}
              </button>
            </div>
          </AlertDialog.Popup>
        </AlertDialog.Portal>
      </AlertDialog.Root>
    </section>
  );
}
