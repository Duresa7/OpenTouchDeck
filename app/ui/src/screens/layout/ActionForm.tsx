import { useState, type ReactNode } from 'react';
import { Switch } from '@base-ui/react/switch';
import { Toggle } from '@base-ui/react/toggle';
import { ToggleGroup } from '@base-ui/react/toggle-group';
import {
  ACTION_GROUPS,
  KEYS,
  MEDIA,
  offersBluetooth,
  MOD_NAMES,
  PROXY_FIELDS,
  spellShortcut,
  splitArgs,
  switchKind,
} from '../../lib/layout/actions.ts';
import { useDevices } from '../../lib/companion/devices.ts';
import { useIntegrations } from '../../lib/companion/integrations.ts';
import type { EditorStore } from '../../lib/editor/store.ts';
import { idWords, streamLabel } from '../../lib/integrations/model.ts';
import { ACTION_TYPES, MODIFIERS, type Action, type GridPage, type Layout, type Tile } from '../../lib/layout/model.ts';
import { useEffectiveTileEdit } from './editor.ts';
import { described, Field, FieldError, Select, useFieldIds } from './fields.tsx';

type Of<T extends Action['type']> = Extract<Action, { type: T }>;

interface KindProps<T extends Action['type']> {
  action: Of<T>;
  commit(next: Action): void;
  error(field: string): string | null;
}

function TextField({ label, value, placeholder, hint, error, onChange, testId }: { label: string; value: string; placeholder?: string; hint?: ReactNode; error: string | null; onChange(value: string): void; testId: string }) {
  const ids = useFieldIds();
  return (
    <Field ids={ids} label={label} hint={hint} error={error}>
      <input
        id={ids.control}
        className="input"
        value={value}
        placeholder={placeholder}
        spellCheck={false}
        autoComplete="off"
        onChange={(event) => onChange(event.target.value)}
        data-testid={testId}
        {...described(ids, hint, error)}
      />
    </Field>
  );
}

function ArgsField({ label, args, error, onChange }: { label: string; args: string[]; error: string | null; onChange(args: string[]): void }) {
  const ids = useFieldIds();
  return (
    <Field ids={ids} label={label} error={error}>
      <textarea
        id={ids.control}
        className="input textarea"
        rows={2}
        value={args.join('\n')}
        spellCheck={false}
        onChange={(event) => onChange(splitArgs(event.target.value))}
        data-testid="action-args"
        {...described(ids, null, error)}
      />
    </Field>
  );
}

function SwitchField({ label, checked, onChange, testId }: { label: string; checked: boolean; onChange(on: boolean): void; testId: string }) {
  const ids = useFieldIds();
  return (
    <div className="field">
      <div className="switch-row">
        <span className="field-label" id={ids.control}>
          {label}
        </span>
        <Switch.Root className="switch" checked={checked} onCheckedChange={onChange} aria-labelledby={ids.control} data-testid={testId}>
          <Switch.Thumb className="switch-thumb" />
        </Switch.Root>
      </div>
    </div>
  );
}

function KeyFields({ action, commit, error }: KindProps<'key'>) {
  const ids = useFieldIds();
  const holdId = useFieldIds();
  const known = KEYS.some((key) => key.usage === action.usage);
  return (
    <>
      <Field ids={ids} label="Key" error={error('usage')}>
        <Select id={ids.control} value={action.usage} onChange={(event) => commit({ ...action, usage: Number(event.target.value) })} data-testid="action-key" {...described(ids, null, error('usage'))}>
          {!known && <option value={action.usage}>Key code {action.usage}</option>}
          {KEYS.map((key) => (
            <option key={key.usage} value={key.usage}>
              {key.name}
            </option>
          ))}
        </Select>
      </Field>
      <div className="field">
        <span className="field-label" id={holdId.control}>
          Hold with it
        </span>
        <ToggleGroup
          multiple
          value={action.mods}
          onValueChange={(held) => commit({ ...action, mods: MODIFIERS.filter((mod) => held.includes(mod)) })}
          aria-labelledby={holdId.control}
          className="segmented segmented-fill"
        >
          {MODIFIERS.map((mod) => (
            <Toggle key={mod} value={mod} className="segment" data-testid={`action-mod-${mod}`}>
              {MOD_NAMES[mod].replace(' key', '')}
            </Toggle>
          ))}
        </ToggleGroup>
        <p className="field-hint" data-testid="action-shortcut">
          Presses <strong className="shortcut">{spellShortcut(action)}</strong> {action.sink === 'bluetooth' ? 'on the Bluetooth computer or phone' : 'on this PC'}.
        </p>
      </div>
    </>
  );
}

function ModFields({ action, commit }: KindProps<'mod'>) {
  const ids = useFieldIds();
  const hint = 'Tap once to hold it for the next key. Double-tap to lock it until you tap again.';
  return (
    <Field ids={ids} label="Key" hint={hint}>
      <Select id={ids.control} value={action.mod} onChange={(event) => commit({ ...action, mod: MODIFIERS.find((mod) => mod === event.target.value) ?? action.mod })} data-testid="action-modifier" {...described(ids, hint, null)}>
        {MODIFIERS.map((mod) => (
          <option key={mod} value={mod}>
            {MOD_NAMES[mod]}
          </option>
        ))}
      </Select>
    </Field>
  );
}

function MediaFields({ action, commit, error }: KindProps<'consumer'>) {
  const ids = useFieldIds();
  const known = MEDIA.some((key) => key.usage === action.usage);
  return (
    <Field ids={ids} label="Key" error={error('usage')}>
      <Select id={ids.control} value={action.usage} onChange={(event) => commit({ ...action, usage: Number(event.target.value) })} data-testid="action-media" {...described(ids, null, error('usage'))}>
        {!known && <option value={action.usage}>Media code {action.usage}</option>}
        {MEDIA.map((key) => (
          <option key={key.usage} value={key.usage}>
            {key.name}
          </option>
        ))}
      </Select>
    </Field>
  );
}

function PageFields({ action, commit, error, pages }: KindProps<'page'> & { pages: Layout['pages'] }) {
  const ids = useFieldIds();
  return (
    <Field ids={ids} label="Page" error={error('page')}>
      <Select id={ids.control} value={action.page} onChange={(event) => commit({ ...action, page: event.target.value })} data-testid="action-page" {...described(ids, null, error('page'))}>
        {pages.map((page) => (
          <option key={page.id} value={page.id}>
            {page.title}
          </option>
        ))}
      </Select>
    </Field>
  );
}

function LaunchFields({ action, commit, error }: KindProps<'launch'>) {
  const [more, setMore] = useState(action.args.length > 0 || action.cwd !== '');
  const open = more || error('args') !== null || error('cwd') !== null;
  return (
    <>
      <TextField label="App" value={action.path} placeholder="notepad.exe or a full path" error={error('path')} onChange={(path) => commit({ ...action, path })} testId="action-path" />
      <details className="inspector-more" open={open} onToggle={(event) => setMore(event.currentTarget.open)}>
        <summary>More options</summary>
        <div className="inspector-more-body">
          <ArgsField label="Arguments, one per line" args={action.args} error={error('args')} onChange={(args) => commit({ ...action, args })} />
          <TextField label="Start in folder" value={action.cwd} error={error('cwd')} onChange={(cwd) => commit({ ...action, cwd })} testId="action-cwd" />
        </div>
      </details>
    </>
  );
}

function OpenFields({ action, commit, error }: KindProps<'open'>) {
  return (
    <TextField
      label="File, folder or web address"
      value={action.target}
      placeholder="https://example.com"
      hint="Opens it as a double-click would."
      error={error('target')}
      onChange={(target) => commit({ ...action, target })}
      testId="action-target"
    />
  );
}

function CommandFields({ action, commit, error }: KindProps<'command'>) {
  return (
    <>
      <TextField label="Program" value={action.program} error={error('program')} onChange={(program) => commit({ ...action, program })} testId="action-program" />
      <ArgsField label="Arguments, one per line" args={action.args} error={error('args')} onChange={(args) => commit({ ...action, args })} />
      <TextField label="Run in folder" value={action.cwd} placeholder="Open TouchDeck's folder" error={error('cwd')} onChange={(cwd) => commit({ ...action, cwd })} testId="action-cwd" />
      <SwitchField label="Run through the system shell" checked={action.shell} onChange={(shell) => commit({ ...action, shell })} testId="action-shell" />
    </>
  );
}

const PROXY = 'cliproxyapi';

function WidgetFields({ action, commit, error }: KindProps<'widget'>) {
  const source = useFieldIds();
  const field = useFieldIds();
  const integrations = useIntegrations((s) => s.integrations);
  const proxy = action.source === PROXY;
  // As before, a Live tile keeps its Integration or turns into a CLIProxyAPI one; now each is named for people.
  const sources = integrations.filter((i) => i.id === PROXY || i.id === action.source);
  const current = integrations.find((i) => i.id === action.source);
  const streams = current?.streams ?? [];
  // Another Integration starts on its first reading, so the Tile never names one the new Integration lacks.
  const pick = (next: string) => {
    const first = next === PROXY ? Object.keys(PROXY_FIELDS)[0] : integrations.find((i) => i.id === next)?.streams[0]?.id;
    commit({ ...action, source: next, field: first ?? action.field, account: next === PROXY ? action.account : '' });
  };
  return (
    <>
      <Field ids={source} label="Integration" error={error('source')}>
        <Select id={source.control} value={action.source} onChange={(event) => pick(event.target.value)} data-testid="action-source">
          {!sources.some((i) => i.id === action.source) && <option value={action.source}>{current?.name ?? idWords(action.source)}</option>}
          {sources.map((integration) => (
            <option key={integration.id} value={integration.id}>
              {integration.name}
            </option>
          ))}
          {!sources.some((i) => i.id === PROXY) && <option value={PROXY}>CLIProxyAPI</option>}
        </Select>
      </Field>
      <Field ids={field} label="Show" error={error('field')}>
        {proxy ? (
          <Select id={field.control} value={action.field} onChange={(event) => commit({ ...action, field: event.target.value })} data-testid="action-field">
            {!(action.field in PROXY_FIELDS) && <option value={action.field}>{action.field} (unknown)</option>}
            {Object.entries(PROXY_FIELDS).map(([value, name]) => (
              <option key={value} value={value}>
                {name}
              </option>
            ))}
          </Select>
        ) : streams.length > 0 && current ? (
          <Select id={field.control} value={action.field} onChange={(event) => commit({ ...action, field: event.target.value })} data-testid="action-field">
            {!streams.some((s) => s.id === action.field) && <option value={action.field}>{idWords(action.field)}</option>}
            {streams.map((stream) => (
              <option key={stream.id} value={stream.id}>
                {streamLabel(current, stream.id)}
              </option>
            ))}
          </Select>
        ) : (
          <input id={field.control} className="input" value={action.field} spellCheck={false} onChange={(event) => commit({ ...action, field: event.target.value })} data-testid="action-field" />
        )}
      </Field>
      {proxy && (
        <TextField
          label="Account"
          value={action.account}
          placeholder="All accounts"
          error={error('account')}
          onChange={(account) => commit({ ...action, account })}
          testId="action-account"
        />
      )}
    </>
  );
}

function KindFields({ action, commit, error, pages }: { action: Action; commit(next: Action): void; error(field: string): string | null; pages: Layout['pages'] }) {
  switch (action.type) {
    case 'key':
      return <KeyFields action={action} commit={commit} error={error} />;
    case 'mod':
      return <ModFields action={action} commit={commit} error={error} />;
    case 'consumer':
      return <MediaFields action={action} commit={commit} error={error} />;
    case 'page':
      return <PageFields action={action} commit={commit} error={error} pages={pages} />;
    case 'launch':
      return <LaunchFields action={action} commit={commit} error={error} />;
    case 'open':
      return <OpenFields action={action} commit={commit} error={error} />;
    case 'command':
      return <CommandFields action={action} commit={commit} error={error} />;
    case 'widget':
      return <WidgetFields action={action} commit={commit} error={error} />;
    case 'none':
      return <p className="field-hint">The Tile shows but does nothing. Use it as a label.</p>;
    default: {
      const unknown: never = action;
      return unknown;
    }
  }
}

export function ActionForm({ store, page, tile, layout, device, deviceId }: { store: EditorStore; page: GridPage; tile: Tile; layout: Layout; device: string; deviceId: string }) {
  const bluetooth = useDevices((s) => offersBluetooth(s.devices.find((d) => d.id === deviceId)));
  const sinkIds = useFieldIds();
  const edit = useEffectiveTileEdit(store, device, tile.id);
  const ids = useFieldIds();
  if (!edit) return null;
  const { value: action, error: message, field: at } = edit.action;
  const pageIds = layout.pages.map((p) => p.id);
  const commit = (next: Action) => void store.editAction(tile.id, next);
  const error = (field: string) => (at === field ? message : null);
  const sinkError = bluetooth === false && (action.type === 'key' || action.type === 'consumer') && action.sink === 'bluetooth'
    ? "This Device can't send to Bluetooth. Choose This PC (Companion)."
    : null;
  return (
    <div className="action-form" data-testid="action-form" data-type={action.type} data-invalid={message !== null ? 'true' : undefined}>
      <Field ids={ids} label="When tapped">
        <Select
          id={ids.control}
          value={action.type}
          onChange={(event) => {
            const kind = ACTION_TYPES.find((type) => type === event.target.value);
            if (kind) commit(switchKind(action, kind, pageIds, page.id));
          }}
          data-testid="action-type"
        >
          {ACTION_GROUPS.map((group) => (
            <optgroup key={group.name} label={group.name}>
              {group.kinds.map((kind) => (
                <option key={kind.type} value={kind.type}>
                  {kind.name}
                </option>
              ))}
            </optgroup>
          ))}
          {action.type === 'widget' && <option value="widget">Live tile</option>}
        </Select>
      </Field>
      {(action.type === 'key' || action.type === 'consumer') && (bluetooth || action.sink === 'bluetooth') && (
        <Field ids={sinkIds} label="Send to" error={sinkError}>
          <Select id={sinkIds.control} value={action.sink ?? 'link'} onChange={(event) => {
            // The Companion is the default, so it is left out: older Firmware rejects a `sink` field.
            const { sink: _sink, ...rest } = action;
            commit(event.target.value === 'bluetooth' ? { ...rest, sink: 'bluetooth' } : rest);
          }} data-testid="action-sink" {...described(sinkIds, null, sinkError)}>
            <option value="link">This PC (Companion)</option>
            <option value="bluetooth">Bluetooth computer or phone</option>
          </Select>
        </Field>
      )}
      <KindFields key={action.type} action={action} commit={commit} error={error} pages={layout.pages} />
      {edit.unplacedError && <FieldError id={`${ids.error}-form`} error={edit.unplacedError} />}
    </div>
  );
}
