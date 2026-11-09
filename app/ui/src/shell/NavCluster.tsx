import type { ComponentType } from 'react';
import clsx from 'clsx';
import { Tooltip } from '@base-ui/react/tooltip';
import { Cable, House, LayoutGrid, Settings, type LucideProps } from 'lucide-react';
import { useUi, type Section } from '../state/ui.ts';
import { UpdateSlot } from './UpdateSlot.tsx';

const destinations: { section: Section; label: string; icon: ComponentType<LucideProps> }[] = [
  { section: 'home', label: 'Home', icon: House },
  { section: 'layout', label: 'Layout', icon: LayoutGrid },
  { section: 'integrations', label: 'Integrations', icon: Cable },
  { section: 'settings', label: 'Settings', icon: Settings },
];

function NavButton({ label, icon: Icon, current, onClick }: { label: string; icon: ComponentType<LucideProps>; current: boolean; onClick(): void }) {
  return (
    <Tooltip.Root>
      <Tooltip.Trigger className={clsx('nav-button', current && 'is-current')} aria-label={label} aria-current={current ? 'page' : undefined} onClick={onClick}>
        <Icon size={18} strokeWidth={1.9} aria-hidden />
      </Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Positioner className="tooltip-positioner" side="top" sideOffset={8}>
          <Tooltip.Popup className="tooltip">{label}</Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

/**
 * The bottom-centre cluster: Home, Layout, Integrations and Settings, then the
 * Update slot in its own puck. Names appear as tooltips after 500ms, and at
 * once while the pointer stays in the cluster.
 */
export function NavCluster() {
  const section = useUi((s) => s.section);
  const go = useUi((s) => s.go);
  const openSettings = useUi((s) => s.openSettings);

  return (
    <Tooltip.Provider delay={500} closeDelay={0}>
      <nav className="nav-dock" aria-label="Main">
        <div className="nav-puck">
          {destinations.map((d) => (
            <NavButton
              key={d.section}
              label={d.label}
              icon={d.icon}
              current={section === d.section}
              // Settings opens on General, wherever it was left, as the nav's way in.
              onClick={() => (d.section === 'settings' ? openSettings() : go(d.section))}
            />
          ))}
        </div>
        <UpdateSlot />
      </nav>
    </Tooltip.Provider>
  );
}
