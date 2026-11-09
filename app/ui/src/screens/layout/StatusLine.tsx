import { useEffect } from 'react';
import clsx from 'clsx';
import type { EditorState, EditorStore } from '../../lib/editor/store.ts';
import { showError } from '../../shell/error-card.ts';
import { useEditorState } from './editor.ts';

/** Errors already shown as a card, so coming back to the editor doesn't show one again. */
const carded = new WeakSet<NonNullable<EditorState['status']>>();

/**
 * The editor's status line. An error pops up as a card instead, with its detail kept for the Errors page in Settings;
 * the line keeps its words for screen readers only.
 */
export function StatusLine({ store, device }: { store: EditorStore; device: string }) {
  const status = useEditorState(store, (s) => s.status);
  useEffect(() => {
    if (!status || status.kind !== 'error' || carded.has(status)) return;
    carded.add(status);
    showError({ where: `Layout · ${device}`, text: status.text, detail: status.detail ?? null });
  }, [status, device]);
  const hidden = status?.event === 'pulled' || status?.kind === 'error';
  return (
    <div
      className={clsx('editor-status', status && `is-${status.kind}`)}
      role="status"
      data-testid="status"
      data-event={status?.event}
      data-kind={status?.kind}
      data-pages={status?.pages}
      data-tiles={status?.tiles}
    >
      {status && (
        <>
          {/* A load needs no line on screen: the canvas shows it. An error has its card. Screen readers still hear both. */}
          <span className={clsx('editor-status-text', hidden && 'visually-hidden')}>{status.text}</span>
        </>
      )}
    </div>
  );
}
