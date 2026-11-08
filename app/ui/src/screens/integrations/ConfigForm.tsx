import { useId, type ReactNode } from 'react';
import { useStore } from 'zustand';
import type { DetailStore } from '../../lib/integrations/detail.ts';
import { outcomeAt, sameValues, transportWarning, type FormField } from '../../lib/integrations/model.ts';
import { button } from '../../shell/button.ts';

function Field({ store, field, warnKey }: { store: DetailStore; field: FormField; warnKey: boolean }) {
  const id = useId();
  const value = useStore(store, (s) => s.values[field.name]);
  const error = useStore(store, (s) => s.errors[field.name]);
  const text = typeof value === 'string' ? value : '';
  const warning = warnKey && field.kind === 'string' ? transportWarning(text) : null;
  const numeric = field.kind === 'integer' || field.kind === 'number';
  const input = field.kind === 'json' ? (
    <textarea id={id} className="input textarea" rows={12} value={text} spellCheck={false} autoComplete="off" aria-invalid={error ? true : undefined} aria-describedby={error ? `${id}-error` : undefined} onChange={(event) => store.setValue(field.name, event.target.value)} data-testid="config-field" data-field={field.name} />
  ) : (
    <input
      id={id}
      className="input tabular"
      value={text}
      inputMode={field.kind === 'integer' ? 'numeric' : field.kind === 'number' ? 'decimal' : undefined}
      placeholder={field.placeholder}
      spellCheck={false}
      autoComplete="off"
      aria-invalid={error ? true : undefined}
      aria-describedby={error ? `${id}-error` : undefined}
      onChange={(event) => store.setValue(field.name, event.target.value)}
      data-testid="config-field"
      data-field={field.name}
    />
  );
  return (
    <div className="field">
      <label className="field-label" htmlFor={id}>
        {field.label}
      </label>
      {numeric && field.unit ? (
        <span className="input-unit">
          {input}
          <span>{field.unit}</span>
        </span>
      ) : (
        input
      )}
      {error && (
        <p className="field-error" id={`${id}-error`} data-testid="field-error">
          {error}
        </p>
      )}
      {warning && (
        <p className="notice-signal" role="status" data-testid="transport-warning">
          {warning}
        </p>
      )}
    </div>
  );
}

function rows(fields: readonly FormField[]): FormField[][] {
  const out: FormField[][] = [];
  for (let i = 0; i < fields.length; i++) {
    const [field, next] = [fields[i], fields[i + 1]];
    if (field.kind === 'string' && (next?.kind === 'integer' || next?.kind === 'number')) {
      out.push([field, next]);
      i++;
    } else {
      out.push([field]);
    }
  }
  return out;
}

/**
 * The settings form the Integration's schema builds, with its own Save at the bottom right. Secret fields have their
 * own controls, and the switch writes `enabled`, so Save never sends either. Save waits while the switch is writing.
 */
export function ConfigForm({ store, switching = false, outcome }: { store: DetailStore; switching?: boolean; outcome?: ReactNode }) {
  const fields = useStore(store, (s) => s.fields);
  const hasSecrets = useStore(store, (s) => s.secretFields.length > 0);
  const dirty = useStore(store, (s) => !sameValues(s.fields, s.values, s.saved));
  const busy = useStore(store, (s) => s.busy);
  const said = useStore(store, (s) => s.notice !== null && outcomeAt(s.notice.event) === 'settings');
  const blocked = !dirty || busy !== null || switching;
  return (
    <form
      className="config-form"
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        if (!blocked) void store.save();
      }}
      data-testid="config-form"
    >
      {rows(fields).map((row) =>
        row.length === 2 ? (
          <div className="form-row" key={row[0].name}>
            <Field store={store} field={row[0]} warnKey={hasSecrets} />
            <Field store={store} field={row[1]} warnKey={hasSecrets} />
          </div>
        ) : (
          <Field key={row[0].name} store={store} field={row[0]} warnKey={hasSecrets} />
        ),
      )}
      <div className="form-actions is-end">
        <span className="form-outcome">
          {said || !dirty ? (
            outcome
          ) : (
            <span className="form-state" data-testid="config-dirty">
              Unsaved changes
            </span>
          )}
        </span>
        <button type="submit" className={button({ kind: 'primary' })} aria-disabled={blocked} disabled={busy === 'save'} data-testid="config-save">
          {busy === 'save' ? 'Saving…' : 'Save'}
        </button>
      </div>
    </form>
  );
}
