import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { AlertDialog } from '@base-ui/react/alert-dialog';
import { KeyRound } from 'lucide-react';
import { button } from './button.ts';
import './stored-field.css';

/** What a field keeps: a value it shows, or a secret it only says is set. */
export type Stored =
  | { kind: 'plain'; value: string | null }
  | { kind: 'secret'; set: boolean; label?: string };

export interface StoredFieldProps {
  label: string;
  hint: ReactNode;
  stored: Stored;
  busy: boolean;
  /** The field's own problem, shown under it. */
  error?: string | null;
  /** Prefixes every test id: `{testid}-field`, `{testid}-input`, `{testid}-save` and so on. */
  testid: string;
  autoComplete?: string;
  /** More buttons beside Replace once a value is kept. */
  extra?: ReactNode;
  /** The Forget confirm: its question and what stops. */
  forget: { title: string; body: string };
  onSave(value: string): Promise<boolean>;
  onForget(): void;
}

const isSet = (stored: Stored) => (stored.kind === 'plain' ? stored.value !== null : stored.set);

/**
 * A value set once, then shown only by its state, with Replace and Forget. The input is uncontrolled, so what's typed
 * stays in the field until Save sends it. A secret leaves the field before the answer comes back.
 */
export function StoredField({ label, hint, stored, busy, error, testid, autoComplete = 'off', extra, forget, onSave, onForget }: StoredFieldProps) {
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  const replaceButton = useRef<HTMLButtonElement>(null);
  const [replacing, setReplacing] = useState(false);
  const [filled, setFilled] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const set = isSet(stored);
  const editing = !set || replacing;
  const secret = stored.kind === 'secret';

  useEffect(() => {
    if (replacing) input.current?.focus();
  }, [replacing]);

  const save = async () => {
    const el = input.current;
    if (!el || !el.value.trim() || busy) return;
    const value = el.value;
    if (secret) el.value = '';
    setFilled(false);
    if (await onSave(value)) {
      if (input.current) input.current.value = '';
      setReplacing(false);
    } else if (!secret) {
      setFilled(true);
    }
  };

  const cancel = () => {
    if (input.current) input.current.value = '';
    setFilled(false);
    setReplacing(false);
    requestAnimationFrame(() => replaceButton.current?.focus());
  };

  const keys = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      void save();
    } else if (event.key === 'Escape' && replacing) {
      event.stopPropagation();
      cancel();
    }
  };

  return (
    <div className="field" data-testid={`${testid}-field`} data-set={set}>
      <label className="field-label" htmlFor={editing ? id : undefined} id={`${id}-label`}>
        {label}
      </label>
      {editing ? (
        <div className="stored-row">
          <input
            ref={input}
            id={id}
            type={secret ? 'password' : 'text'}
            className="input"
            placeholder={set ? undefined : 'Not set'}
            autoComplete={autoComplete}
            spellCheck={false}
            aria-invalid={error ? true : undefined}
            aria-describedby={error ? `${id}-error` : `${id}-hint`}
            onInput={(event) => setFilled(event.currentTarget.value.trim().length > 0)}
            onKeyDown={keys}
            data-testid={`${testid}-input`}
          />
          <button type="button" className={button()} aria-disabled={!filled || busy} onClick={() => filled && void save()} data-testid={`${testid}-save`}>
            Save
          </button>
          {replacing && (
            <button type="button" className={button()} onClick={cancel} data-testid={`${testid}-cancel`}>
              Cancel
            </button>
          )}
        </div>
      ) : (
        <div className="stored-row">
          <span className="stored-state" aria-labelledby={`${id}-label`} role="status" data-testid={`${testid}-state`}>
            {secret && <KeyRound size={16} strokeWidth={1.9} aria-hidden />}
            {stored.kind === 'plain' ? stored.value : (stored.label ?? 'Set')}
          </span>
          {extra}
          <button ref={replaceButton} type="button" className={button()} aria-disabled={busy || undefined} onClick={() => !busy && setReplacing(true)} data-testid={`${testid}-replace`}>
            Replace
          </button>
          <button type="button" className={button({ kind: 'danger' })} aria-disabled={busy || undefined} onClick={() => !busy && setConfirming(true)} data-testid={`${testid}-forget`}>
            Forget
          </button>
        </div>
      )}
      {error && (
        <p className="field-error" id={`${id}-error`} data-testid="field-error">
          {error}
        </p>
      )}
      <p className="field-hint" id={`${id}-hint`}>
        {hint}
      </p>
      <AlertDialog.Root open={confirming} onOpenChange={setConfirming}>
        <AlertDialog.Portal>
          <AlertDialog.Backdrop className="scrim" />
          <AlertDialog.Popup className="dialog dialog-confirm" data-testid="forget-dialog">
            <AlertDialog.Title className="dialog-title">{forget.title}</AlertDialog.Title>
            <AlertDialog.Description className="dialog-text">{forget.body}</AlertDialog.Description>
            <div className="dialog-actions">
              <AlertDialog.Close className={button()}>Cancel</AlertDialog.Close>
              <button
                type="button"
                className={button({ kind: 'primary' })}
                onClick={() => {
                  setConfirming(false);
                  onForget();
                }}
                data-testid="forget-confirm"
              >
                Forget
              </button>
            </div>
          </AlertDialog.Popup>
        </AlertDialog.Portal>
      </AlertDialog.Root>
    </div>
  );
}
