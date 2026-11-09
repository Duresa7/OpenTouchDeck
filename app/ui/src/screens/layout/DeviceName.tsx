import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react';
import { useStore } from 'zustand';
import { Pencil } from 'lucide-react';
import type { DeviceSummary } from '../../lib/companion/api.ts';
import { needsFirmwareField } from '../../lib/device/needs-firmware.ts';
import { button } from '../../shell/button.ts';
import { useDeviceStores } from './device.ts';
import { Tip } from './Tip.tsx';

/**
 * The Device's name at the top left of the editor, with a pencil to rename it in place. Enter or leaving the field
 * saves the name to the Device; Escape puts the old one back. The rest of the Device's settings live in Settings.
 */
export function DeviceName({ device }: { device: DeviceSummary }) {
  const { settings } = useDeviceStores(device);
  const live = device.link.state === 'connected';
  const loaded = useStore(settings, (s) => s.load === 'ready');
  const restarting = useStore(settings, (s) => s.restart === 'restarting');
  const shown = useStore(settings, (s) => s.shown?.name ?? null);
  const lacks = useStore(settings, (s) => s.needs.name);
  const problem = useStore(settings, (s) => s.problem.name);
  const value = shown ?? device.name ?? '';
  const name = value || 'Unnamed Device';
  const needs = lacks && needsFirmwareField({ device: name, since: lacks.since });
  const can = live && loaded && !restarting && !needs;
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);
  const pencil = useRef<HTMLButtonElement>(null);
  const back = useRef(false);
  const id = useId();

  useEffect(() => {
    if (live) void settings.load();
  }, [settings, live]);
  // Unplugging or a restart ends a rename that can no longer reach the Device.
  useEffect(() => {
    if (!can) setEditing(false);
  }, [can]);
  // Leaving the field by Enter or Escape puts focus back on the pencil, so the keyboard keeps its place.
  useEffect(() => {
    if (!editing && back.current) pencil.current?.focus();
    back.current = false;
  }, [editing]);

  const start = () => {
    if (!can) return;
    setDraft(value);
    setEditing(true);
  };
  const save = async (refocus: boolean) => {
    const next = draft.trim();
    if (next === value) {
      back.current = refocus;
      setEditing(false);
      return;
    }
    if (await settings.rename(next)) {
      back.current = refocus;
      setEditing(false);
    }
  };
  const keys = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      void save(true);
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      back.current = true;
      setEditing(false);
    }
  };

  const tip = !live ? `Plug in ${name} to rename it` : needs ? needs : `Rename ${name}`;

  if (editing) {
    return (
      <div className="device-title is-editing">
        <input
          className="input device-title-input"
          value={draft}
          autoFocus
          spellCheck={false}
          autoComplete="off"
          aria-label="Device name"
          aria-invalid={problem !== undefined}
          aria-describedby={problem ? `${id}-problem` : undefined}
          onFocus={(event) => event.currentTarget.select()}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={() => void save(false)}
          onKeyDown={keys}
          data-testid="device-rename-input"
        />
        {problem && (
          <p id={`${id}-problem`} className="device-title-problem" role="alert" data-testid="device-rename-problem">
            {problem}
          </p>
        )}
      </div>
    );
  }
  return (
    <div className="device-title">
      <span className="device-title-name" title={name} data-testid="device-name">
        {name}
      </span>
      <Tip label={tip}>
        <button
          ref={pencil}
          type="button"
          className={button({ kind: 'icon' })}
          aria-label={`Rename ${name}`}
          aria-disabled={!can}
          onClick={start}
          data-testid="device-rename"
        >
          <Pencil size={16} strokeWidth={1.9} aria-hidden />
        </button>
      </Tip>
    </div>
  );
}
