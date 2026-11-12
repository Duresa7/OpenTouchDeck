import { useRef, useState } from 'react';
import clsx from 'clsx';
import { Dialog } from '@base-ui/react/dialog';
import { Tooltip } from '@base-ui/react/tooltip';
import { RefreshCw } from 'lucide-react';
import { useUpdates, type UpdateStep } from '../state/updates.ts';
import { iconLabel, percent } from './update-text.ts';
import { UpdateDialog } from './UpdateDialog.tsx';

const RING = 2 * Math.PI * 9;

function UpdateGlyph({ step }: { step: UpdateStep }) {
  if (step.kind !== 'downloading') {
    return (
      <>
        <RefreshCw size={18} strokeWidth={1.9} aria-hidden />
        <span className="update-dot" />
      </>
    );
  }
  const pct = percent(step);
  return (
    <svg width="22" height="22" viewBox="0 0 22 22" className="update-ring" aria-hidden>
      <circle cx="11" cy="11" r="9" className="update-ring-track" />
      {pct !== null && pct > 0 && (
        <circle cx="11" cy="11" r="9" className="update-ring-fill" strokeDasharray={RING} strokeDashoffset={RING * (1 - pct / 100)} />
      )}
    </svg>
  );
}

export function UpdateSlot() {
  const update = useUpdates((s) => s.update);
  const laterFor = useUpdates((s) => s.laterFor);
  const open = useUpdates((s) => s.dialogOpen);
  const setOpen = useUpdates((s) => s.setDialogOpen);
  const icon = useRef<HTMLSpanElement>(null);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [byKeyboard, setByKeyboard] = useState(false);

  if (!update) return null;
  const { step } = update;
  const label = iconLabel(step);
  const laterChosen = laterFor === update.offer.version;
  const expanded = focused || (hovered && !laterChosen);

  return (
    <Dialog.Root open={open} onOpenChange={(next) => setOpen(next)}>
      <div className={clsx('update-slot', expanded && 'is-expanded', byKeyboard && 'is-instant')}>
        <Tooltip.Root disabled={!laterChosen || focused}>
          <Tooltip.Trigger
            render={<Dialog.Trigger />}
            className="update-button"
            data-step={step.kind}
            aria-label={label}
            onPointerEnter={(event) => {
              if (event.pointerType !== 'mouse') return;
              setByKeyboard(false);
              setHovered(true);
            }}
            onPointerLeave={() => setHovered(false)}
            onFocus={(event) => {
              if (!event.currentTarget.matches(':focus-visible')) return;
              setByKeyboard(true);
              setFocused(true);
            }}
            onBlur={() => setFocused(false)}
          >
            <span className="update-icon" ref={icon}>
              <UpdateGlyph step={step} />
            </span>
            <span className="update-label" aria-hidden>
              {label}
            </span>
          </Tooltip.Trigger>
          <Tooltip.Portal>
            <Tooltip.Positioner className="tooltip-positioner" side="right" sideOffset={8} anchor={icon}>
              <Tooltip.Popup className="tooltip">{label}</Tooltip.Popup>
            </Tooltip.Positioner>
          </Tooltip.Portal>
        </Tooltip.Root>
      </div>
      <UpdateDialog update={update} />
    </Dialog.Root>
  );
}
