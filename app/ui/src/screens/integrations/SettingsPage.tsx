import type { ReactNode, Ref } from 'react';
import clsx from 'clsx';
import { ChevronLeft } from 'lucide-react';
import { button } from '../../shell/button.ts';
import { IntegrationGlyph } from './IntegrationGlyph.tsx';
import { IntegrationSwitch, type OnOff } from './IntegrationRow.tsx';

export interface SettingsPageProps {
  id: string;
  name: string;
  icon: string;
  lede: ReactNode;
  /** The status box and its words under the lede, when there's something to say beyond the switch. */
  status?: ReactNode;
  state: OnOff;
  titleRef?: Ref<HTMLHeadingElement>;
  onBack(): void;
  children: ReactNode;
}

/** An Integration's settings page: Back, its name with the same switch its row has, then its settings. */
export function SettingsPage({ id, name, icon, lede, status, state, titleRef, onBack, children }: SettingsPageProps) {
  return (
    <div className="integration-detail" data-testid="integration-detail" data-id={id}>
      <button type="button" className={clsx(button({ size: 'small' }), 'detail-back')} onClick={onBack} data-testid="detail-back">
        <ChevronLeft size={16} strokeWidth={1.9} aria-hidden />
        Integrations
      </button>
      <header className="detail-head">
        <IntegrationGlyph icon={icon} size={22} />
        <div className="detail-head-text">
          <div className="detail-title-row">
            <h1 className="page-title detail-title" ref={titleRef} tabIndex={-1}>
              {name}
            </h1>
            <IntegrationSwitch name={name} state={state} testId="detail-switch" />
          </div>
          <p className="page-lede">{lede}</p>
          {status}
        </div>
      </header>
      {children}
    </div>
  );
}
