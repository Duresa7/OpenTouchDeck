import { useEffect, useId, useRef } from 'react';
import { Dialog } from '@base-ui/react/dialog';
import { ChevronLeft, ChevronRight, X } from 'lucide-react';
import { useStore } from 'zustand';
import { brandfetchStore, DEVELOPER_PORTAL, isBusy, logoCount, type BrandfetchStep, type BrandfetchStore } from '../../lib/brandfetch/settings.ts';
import { useCompanion } from '../../lib/companion/connection.ts';
import { useKeychainName } from '../../lib/companion/keychain.ts';
import { reloadIntoSettings } from '../../state/ui.ts';
import { button } from '../button.ts';
import { StoredField } from '../StoredField.tsx';
import './brandfetch.css';

/**
 * The Companion opens the window's content policy to Brandfetch only while a client ID is saved,
 * and sets that policy when the page loads. Dev and demo builds run without one, so only a
 * production build reloads.
 */
const RELOADS = !import.meta.env.DEV;

/** What the last change left to say: a notice, or a problem with the Companion's words under Details. */
function StepLine({ step }: { step: BrandfetchStep }) {
  if (step.kind === 'done')
    return (
      <p className="field-hint brandfetch-notice" role="status" data-testid="brandfetch-notice">
        {step.notice}
      </p>
    );
  if (step.kind !== 'failed') return null;
  const { text, detail } = step.problem;
  return (
    <div className="settings-problem brandfetch-problem" role="alert" data-testid="brandfetch-problem">
      <p>{text}</p>
      {detail && detail !== text && (
        <details className="brandfetch-details">
          <summary>Details</summary>
          <p data-testid="brandfetch-problem-detail">{detail}</p>
        </details>
      )}
    </div>
  );
}

function Keys({ store }: { store: BrandfetchStore }) {
  const view = useStore(store, (s) => s.view);
  const id = useId();
  if (view.kind === 'loading') return <p className="field-hint">Reading the Brandfetch settings…</p>;
  if (view.kind === 'unreadable') return <p className="settings-problem">{view.problem}</p>;
  const { status, step } = view;
  const busy = isBusy(view);
  const count = logoCount(status);
  const state = store.getState();

  // Adding or forgetting the client ID changes the page's content policy, so the window reloads.
  const saveClientId = async (value: string) => {
    const added = status.client_id === null;
    const saved = await state.saveClientId(value);
    if (saved && added && RELOADS) reloadIntoSettings('brandfetch');
    return saved;
  };
  const forgetClientId = async () => {
    if ((await state.forgetClientId()) && RELOADS) reloadIntoSettings('brandfetch');
  };

  const test = (
    <button type="button" className={button()} aria-disabled={busy || undefined} onClick={() => !busy && void state.testKey()} data-testid="brandfetch-key-test">
      Test key
    </button>
  );
  return (
    <>
      <section className="settings-section" aria-labelledby={`${id}-keys`}>
        <h3 id={`${id}-keys`}>Keys</h3>
        <div className="group brandfetch-keys">
          <StoredField
            label="Client ID"
            hint="Turns on logo search in the icon picker. Searches go from this window straight to Brandfetch. Adding or forgetting it reloads the window."
            stored={{ kind: 'plain', value: status.client_id }}
            busy={busy}
            testid="brandfetch-client-id"
            forget={{ title: 'Forget the client ID?', body: 'Logo search stops until you add it again.' }}
            onSave={saveClientId}
            onForget={() => void forgetClientId()}
          />
          <StoredField
            label="API key"
            hint="Turns a picked logo into a Tile icon. Each pick uses one Brand API request; Test key is free."
            stored={{ kind: 'secret', set: status.key_set }}
            busy={busy}
            testid="brandfetch-key"
            autoComplete="new-password"
            extra={test}
            forget={{ title: 'Forget the API key?', body: 'Picking Brandfetch logos stops until you add it again.' }}
            onSave={state.saveKey}
            onForget={() => void state.forgetKey()}
          />
          <StepLine step={step} />
        </div>
      </section>
      <section className="settings-section" aria-labelledby={`${id}-logos`}>
        <h3 id={`${id}-logos`}>Logos on this PC</h3>
        <div className="group">
          <div className="row">
            <span className="row-text">
              <span className="row-title" data-testid="brandfetch-count">
                {count ?? 'No Brandfetch logos yet'}
              </span>
              <span className="row-sub" data-testid="brandfetch-expiry">
                {status.expired.length > 0
                  ? `${status.expired.length} went 30 days without a refresh, so this PC deleted their cached copy. Their Tiles keep showing them; pick them again to refresh them.`
                  : 'Each logo is fetched again from day 25, so no cached copy is older than 30 days.'}
              </span>
            </span>
          </div>
        </div>
      </section>
    </>
  );
}

/** The General view's row that opens this section, in the group beside Icon packs. */
export function BrandfetchEntry({ onOpen, focus }: { onOpen(): void; focus: boolean }) {
  const companion = useCompanion();
  const store = brandfetchStore(companion);
  const view = useStore(store, (s) => s.view);
  const entry = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    void store.getState().load();
  }, [store]);
  useEffect(() => {
    if (focus) entry.current?.focus();
  }, [focus]);
  const on = view.kind === 'ready' && view.status.client_id !== null;
  return (
    <button ref={entry} type="button" className="row row-button" onClick={onOpen} data-testid="brandfetch-entry">
      <span className="row-text">
        <span className="row-title">Brandfetch logos</span>
        <span className="row-sub">{on ? 'On. The icon picker searches Brandfetch with your client ID.' : 'Off. Add your own Brandfetch keys to search brand logos.'}</span>
      </span>
      <ChevronRight className="row-chevron" size={18} strokeWidth={1.9} aria-hidden />
    </button>
  );
}

export function BrandfetchView({ onBack, focusBack }: { onBack(): void; focusBack: boolean }) {
  const companion = useCompanion();
  const store = brandfetchStore(companion);
  const back = useRef<HTMLButtonElement>(null);
  const id = useId();
  const keychain = useKeychainName();
  useEffect(() => {
    void store.getState().load();
  }, [store]);
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
          <Dialog.Title className="dialog-title">Brandfetch logos</Dialog.Title>
        </div>
        <Dialog.Close className={button({ kind: 'icon' })} aria-label="Close">
          <X size={18} strokeWidth={1.9} aria-hidden />
        </Dialog.Close>
      </div>
      <p className="dialog-lede">Search brand logos in the icon picker with your own Brandfetch account. It stays off until you add a client ID.</p>
      <Keys store={store} />
      <section className="settings-section" aria-labelledby={`${id}-stored`}>
        <h3 id={`${id}-stored`}>How the key is stored</h3>
        <p className="brandfetch-stored" data-testid="brandfetch-stored">
          The API key is stored in {keychain}, never in a file. Open TouchDeck never shows it again and sends it only to Brandfetch's Brand API, when you pick a logo, test
          the key or a logo is refreshed.
        </p>
      </section>
      <section className="settings-section" aria-labelledby={`${id}-terms`}>
        <h3 id={`${id}-terms`}>Good to know</h3>
        <ul className="brandfetch-notes" data-testid="brandfetch-notes">
          <li>You use your own Brandfetch account and keys. Open TouchDeck ships none.</li>
          <li>Logos stay the trademarks of their brand owners.</li>
          <li>Brandfetch lets apps keep a logo for 30 days. This PC fetches each one again before then; the free developer plan includes about 100 Brand API requests.</li>
        </ul>
        <a className="brandfetch-portal" href={DEVELOPER_PORTAL} target="_blank" rel="noreferrer">
          Get a client ID and API key from Brandfetch
        </a>
      </section>
    </>
  );
}
