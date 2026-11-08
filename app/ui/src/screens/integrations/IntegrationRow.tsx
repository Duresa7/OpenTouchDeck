import type { ReactNode, Ref } from 'react';
import clsx from 'clsx';
import { Switch } from '@base-ui/react/switch';
import { Settings } from 'lucide-react';
import { button } from '../../shell/button.ts';
import { Tip } from '../layout/Tip.tsx';
import { IntegrationGlyph } from './IntegrationGlyph.tsx';

export interface OnOff {
  checked: boolean;
  /** Shown as waiting: a write is on its way, or an enable waits for setup. */
  busy: boolean;
  /** Can't be flipped: a write is on its way or queued. */
  locked: boolean;
  flip(on: boolean): void;
}

/** An Integration's on/off switch: it saves the moment it's flipped. While an enable waits for setup, off undoes it. */
export function IntegrationSwitch({ name, state, testId = 'integration-switch' }: { name: string; state: OnOff; testId?: string }) {
  return (
    <Switch.Root
      className="integration-switch"
      checked={state.checked}
      readOnly={state.locked}
      onCheckedChange={(on) => state.flip(on)}
      aria-label={`Turn on ${name}`}
      aria-busy={state.busy || undefined}
      data-busy={state.busy || undefined}
      data-testid={testId}
    >
      <Switch.Thumb className="integration-switch-thumb" />
    </Switch.Root>
  );
}

export interface IntegrationRowProps {
  id: string;
  name: string;
  icon: string;
  /** The line under the name: what it does, or what's wrong. */
  line: ReactNode;
  /** Opens the settings page, for an Integration that has one. */
  onSettings?: () => void;
  settingsRef?: Ref<HTMLButtonElement>;
  state: OnOff;
  /** Extra data attributes for specs and styling. */
  data?: Record<`data-${string}`, string | number | boolean | undefined>;
}

/** One Integration: what it is, where its settings are, and whether it's on. The row itself is not a button. */
export function IntegrationRow({ id, name, icon, line, onSettings, settingsRef, state, data }: IntegrationRowProps) {
  return (
    <li className="integration" data-testid="integration" data-id={id} data-on={state.checked} {...data}>
      <IntegrationGlyph icon={icon} />
      <span className="integration-text">
        <span className="integration-name">{name}</span>
        <span className="integration-line">{line}</span>
      </span>
      <span className={clsx('integration-gear', !onSettings && 'is-empty')}>
        {onSettings && (
          <Tip label="Settings">
            <button ref={settingsRef} type="button" className={button({ kind: 'icon' })} aria-label={`${name} settings`} onClick={onSettings} data-testid="integration-settings">
              <Settings size={18} strokeWidth={1.9} aria-hidden />
            </button>
          </Tip>
        )}
      </span>
      <IntegrationSwitch name={name} state={state} />
    </li>
  );
}
