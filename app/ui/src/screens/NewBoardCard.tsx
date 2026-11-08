import { useEffect, useId } from 'react';
import clsx from 'clsx';
import { useStore } from 'zustand';
import { Tooltip } from '@base-ui/react/tooltip';
import { Cpu } from 'lucide-react';
import type { NewBoard } from '../lib/companion/api.ts';
import { useCompanion } from '../lib/companion/connection.ts';
import { boardStep } from '../lib/device/new-board.ts';
import { button } from '../shell/button.ts';
import { setupUi } from '../state/setup.ts';
import { DevicePreview } from './DevicePreview.tsx';
import { boardInstallFor, boardsUi } from './new-boards.ts';

export function NewBoardCard({ board, width }: { board: NewBoard; width: number }) {
  const companion = useCompanion();
  const install = boardInstallFor(companion, board);
  const view = useStore(install, (s) => s.view);
  const log = useStore(install, (s) => s.log);
  const step = boardStep(view);
  const ids = useId();

  useEffect(() => {
    if (step === 'done') setupUi.getState().unskip(board.id);
  }, [step, board.id]);

  const start = () => {
    boardsUi.getState().start(board);
    void install.start('bundled');
  };
  const cancel = () => {
    install.dismiss();
    boardsUi.getState().end(board.id);
  };

  return (
    <li className="device-figure board-figure" style={{ width }} data-testid="new-board" data-id={board.id} data-step={step}>
      <div className="board-ghost">
        <DevicePreview preview={null} live={step === 'done'} />
      </div>
      <div className="device-caption board-caption" role="group" aria-labelledby={`${ids}-title`}>
        <span className="device-name" id={`${ids}-title`}>
          {step === 'done' ? 'Firmware installed' : 'New board found'}
        </span>
        {step === 'detected' && (
          <>
            <p className="board-text">The board on {board.port} doesn't run Open TouchDeck Firmware yet.</p>
            <span className="board-buttons">
              <button type="button" className={button({ kind: 'primary' })} onClick={start} data-testid="board-install">
                Install Firmware
              </button>
              <button type="button" className={button()} onClick={() => boardsUi.getState().ignore(board.id)} data-testid="board-ignore">
                Ignore
              </button>
            </span>
          </>
        )}
        {view.step === 'waiting' && (
          <span className="status is-waiting" role="status" data-testid="board-status">
            <i className="status-box" data-tone="wait" aria-hidden />
            {view.text}
          </span>
        )}
        {view.step === 'running' && (
          <>
            <span className="board-progress-text tabular" role="status" data-testid="board-status">
              {view.text}
            </span>
            <div
              className="board-progress"
              role="progressbar"
              aria-labelledby={`${ids}-title`}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={view.pct ?? undefined}
              aria-valuetext={view.text}
            >
              <span style={{ transform: `scaleX(${(view.pct ?? 0) / 100})` }} />
            </div>
            <p className="board-text">Keep the board plugged in. This takes about a minute.</p>
          </>
        )}
        {step === 'done' && (
          <p className="board-text" role="status" data-testid="board-status">
            Next, name your Device.
          </p>
        )}
        {view.step === 'failed' && (
          <>
            <p className="board-text is-error" role="alert" data-testid="board-status" data-code={view.code}>
              {view.text}
            </p>
            <details className="board-details">
              <summary>Details</summary>
              <p>{log.join('\n')}</p>
            </details>
            <span className="board-buttons">
              <button type="button" className={button({ kind: 'primary' })} onClick={start} data-testid="board-retry">
                Retry
              </button>
              <button type="button" className={button()} onClick={cancel} data-testid="board-cancel">
                Cancel
              </button>
            </span>
          </>
        )}
      </div>
    </li>
  );
}

export function IgnoredBoards({ boards }: { boards: NewBoard[] }) {
  if (!boards.length) return null;
  return (
    <Tooltip.Provider delay={500} closeDelay={0}>
      <ul className="ignored-boards" aria-label="Ignored New boards">
        {boards.map((board) => (
          <li key={board.id}>
            <Tooltip.Root>
              <Tooltip.Trigger
                className={clsx(button({ size: 'small' }), 'ignored-board')}
                onClick={() => boardsUi.getState().restore(board.id)}
                data-testid="board-ignored"
                data-id={board.id}
              >
                <Cpu size={14} strokeWidth={1.9} aria-hidden />
                New board
              </Tooltip.Trigger>
              <Tooltip.Portal>
                <Tooltip.Positioner side="top" sideOffset={8}>
                  <Tooltip.Popup className="tooltip">Ignored. Click to install Firmware.</Tooltip.Popup>
                </Tooltip.Positioner>
              </Tooltip.Portal>
            </Tooltip.Root>
          </li>
        ))}
      </ul>
    </Tooltip.Provider>
  );
}
