import { useId, useMemo, useRef, useState } from 'react';
import clsx from 'clsx';
import { Popover } from '@base-ui/react/popover';
import type { EditorStore } from '../../lib/editor/store.ts';
import { assetOf, type ActionType, type Tile } from '../../lib/layout/model.ts';
import { readRecent, recentKey } from '../../lib/icons/recent.ts';
import { button } from '../../shell/button.ts';
import { windowTop } from '../../shell/WindowFrame.tsx';
import { TileIcon } from './AssetImage.tsx';
import { SYMBOL_NAMES } from './glyphs.tsx';
import { IconPicker } from './IconPicker.tsx';

function storage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

function iconName(tile: Pick<Tile, 'icon' | 'asset'>): string {
  const asset = assetOf(tile);
  const key = asset ? `asset:${asset}` : tile.icon;
  if (key === null) return 'No icon';
  if (SYMBOL_NAMES[key]) return SYMBOL_NAMES[key];
  return readRecent(storage()).find((icon) => recentKey(icon) === key)?.name ?? 'Custom icon';
}

export function IconField({ store, tile, kind }: { store: EditorStore; tile: Tile; kind: ActionType }) {
  const label = useId();
  const [open, setOpen] = useState(false);
  const popup = useRef<HTMLDivElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const name = useMemo(() => iconName(tile), [tile, open]);
  const empty = tile.icon === null && assetOf(tile) === null;
  return (
    <div className="field">
      <span className="field-label" id={label}>
        Icon
      </span>
      <div className="icon-field" data-testid="tile-icon">
        <span className="icon-chip" aria-hidden>
          <TileIcon tile={tile} kind={kind} size={18} />
        </span>
        <span className={clsx('icon-name', empty && 'is-empty')} title={name} data-testid="tile-icon-name">
          {name}
        </span>
        <Popover.Root open={open} onOpenChange={setOpen}>
          <Popover.Trigger className={button({ size: 'small' })} aria-describedby={label} data-testid="icon-change">
            Change
          </Popover.Trigger>
          <Popover.Portal>
            <Popover.Positioner
              className="popover-positioner"
              side="bottom"
              align="end"
              sideOffset={8}
              collisionPadding={{ top: 12 + windowTop(), right: 12, bottom: 12, left: 12 }}
              collisionAvoidance={{ side: 'flip', align: 'shift', fallbackAxisSide: 'none' }}
            >
              <Popover.Popup
                ref={popup}
                className="popover icon-picker"
                initialFocus={(type) => (type === 'keyboard' ? search.current : popup.current)}
                data-testid="icon-picker"
              >
                <Popover.Title className="visually-hidden">Choose icon</Popover.Title>
                <IconPicker store={store} tile={tile} searchRef={search} onDone={() => setOpen(false)} />
              </Popover.Popup>
            </Popover.Positioner>
          </Popover.Portal>
        </Popover.Root>
      </div>
    </div>
  );
}
