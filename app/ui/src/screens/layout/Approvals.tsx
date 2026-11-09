import { useEffect, useRef, useState } from 'react';
import clsx from 'clsx';
import type { AllowedAction, ApprovalRequest } from '../../lib/companion/api.ts';
import { actionsStore, useActions } from '../../lib/companion/actions.ts';
import { useCompanion } from '../../lib/companion/connection.ts';
import { isApiError, toApiError } from '../../lib/companion/errors.ts';
import { commandLine, escapeInvisible, processOf, sameProcess, type Started } from '../../lib/layout/actions.ts';
import type { Action, Layout } from '../../lib/layout/model.ts';
import { button } from '../../shell/button.ts';
import { openDetails } from '../../state/ui.ts';

const UNREACHABLE = "Can't reach the Companion. Check that Open TouchDeck is running in the tray, then retry.";

function CommandFacts({ program, args, cwd, shell }: Pick<Started, 'program' | 'args' | 'cwd' | 'shell'>) {
  return (
    <dl className="command-facts">
      <dt>Program</dt>
      <dd data-testid="approval-program">{escapeInvisible(program)}</dd>
      <dt>Arguments</dt>
      <dd>
        {args.length ? (
          <ol className="command-args" data-testid="approval-args">
            {args.map((arg, i) => (
              <li key={i}>{escapeInvisible(arg)}</li>
            ))}
          </ol>
        ) : (
          <span className="is-empty">None</span>
        )}
      </dd>
      <dt>Folder</dt>
      <dd className={clsx(!cwd && 'is-empty')}>{cwd ? escapeInvisible(cwd) : "Open TouchDeck's folder"}</dd>
      <dt>System shell</dt>
      <dd>{shell ? 'Yes' : 'No'}</dd>
    </dl>
  );
}

function where(request: ApprovalRequest, layout: Layout, device: string) {
  const page = layout.pages.find((p) => p.id === request.page);
  const tile = page?.kind === 'grid' ? page.tiles.find((t) => t.id === request.tile) : undefined;
  return `${tile?.label ?? request.tile} on the ${page?.title ?? request.page} Page of ${device}`;
}

function Pending({ request, layout, device }: { request: ApprovalRequest; layout: Layout; device: string }) {
  const companion = useCompanion();
  const [problem, setProblem] = useState<string | null>(null);
  const show = () => {
    setProblem(null);
    companion.approvals.show({ id: request.id }).catch((error: unknown) => {
      if (!isApiError(toApiError(error), 'approval.not_found')) setProblem(UNREACHABLE);
    });
  };
  return (
    <section className="approval" data-testid="approval-pending" data-id={request.id} data-tile={request.tile}>
      <p className="approval-title">
        <i className="status-box" data-tone="wait" aria-hidden />
        Waiting for approval on this PC
      </p>
      <p className="approval-where">{where(request, layout, device)}</p>
      <CommandFacts {...request} />
      <div>
        <button type="button" className={button({ size: 'small' })} onClick={show} data-testid="approval-show">
          Show the dialog again
        </button>
      </div>
      {problem && <p className="field-error">{problem}</p>}
    </section>
  );
}

export function PendingApprovals({ deviceId, device, layout }: { deviceId: string; device: string; layout: Layout }) {
  const companion = useCompanion();
  const approvals = useActions((s) => s.approvals);
  useEffect(() => {
    companion.approvals.list().then(
      (listed) => actionsStore.getState().listed(listed),
      () => {},
    );
  }, [companion]);
  const mine = approvals.filter((request) => request.device === deviceId);
  const waiting = mine.length;
  const seen = useRef(0);
  useEffect(() => {
    if (waiting > seen.current) openDetails();
    seen.current = waiting;
  }, [waiting]);
  return (
    <div className="approvals" aria-live="polite" data-testid="approvals">
      {mine.map((request) => (
        <Pending key={request.id} request={request} layout={layout} device={device} />
      ))}
    </div>
  );
}

export function ApprovalNote({ action }: { action: Action }) {
  const started = processOf(action);
  if (!started) return null;
  return (
    <div className="approval-note" data-testid="approval-note">
      <p className="field-hint">
        {started.kind === 'command' ? 'Open TouchDeck shows the exact command and asks before it runs.' : 'Open TouchDeck shows the exact program and asks before it opens it.'}
      </p>
      {started.program.trim() !== '' && (
        <p className="command-line" data-testid="command-line">
          {escapeInvisible(commandLine(started.program, started.args))}
        </p>
      )}
    </div>
  );
}

function AllowedRow({ allowed, mine, onRemoved }: { allowed: AllowedAction; mine: boolean; onRemoved(): void }) {
  const companion = useCompanion();
  const [problem, setProblem] = useState<string | null>(null);
  const notes = [mine ? 'This Tile' : '', allowed.cwd ? `in ${escapeInvisible(allowed.cwd)}` : '', allowed.shell ? 'through the system shell' : ''].filter(Boolean);
  const remove = () => {
    setProblem(null);
    companion.actions.allowed_remove({ hash: allowed.hash }).then(onRemoved, (error: unknown) => {
      if (isApiError(toApiError(error), 'action.not_found')) onRemoved();
      else setProblem(UNREACHABLE);
    });
  };
  return (
    <li className="allowed-row" data-testid="allowed-action" data-hash={allowed.hash} data-mine={mine ? 'true' : undefined}>
      <span className="allowed-text">
        <span className="command-line">{escapeInvisible(commandLine(allowed.program, allowed.args))}</span>
        {notes.length > 0 && <span className="allowed-sub">{notes.join(', ')}</span>}
        {problem && <span className="field-error">{problem}</span>}
      </span>
      <button type="button" className={button({ size: 'small' })} onClick={remove} data-testid="allowed-remove">
        Remove
      </button>
    </li>
  );
}

export function AllowedActions({ action }: { action: Action }) {
  const companion = useCompanion();
  const allowed = useActions((s) => s.allowed);
  const rev = useActions((s) => s.allowedRev);
  const [reads, setReads] = useState(0);
  useEffect(() => {
    let live = true;
    companion.actions.allowed().then(
      (list) => live && actionsStore.getState().setAllowed(list),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [companion, rev, reads]);
  if (!allowed?.length) return null;
  const started = processOf(action);
  const isMine = (entry: AllowedAction) => started !== null && sameProcess(started, entry);
  const sorted = [...allowed].sort((a, b) => Number(isMine(b)) - Number(isMine(a)));
  return (
    <section className="field" aria-label="Always allowed on this PC" data-testid="allowed-actions">
      <h3 className="field-label">Always allowed on this PC</h3>
      <ul className="allowed-list">
        {sorted.map((entry) => (
          <AllowedRow key={entry.hash} allowed={entry} mine={isMine(entry)} onRemoved={() => setReads((n) => n + 1)} />
        ))}
      </ul>
    </section>
  );
}
