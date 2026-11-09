import { toast } from 'sonner';
import { errorsStore, type ErrorEntry } from '../state/errors.ts';
import { uiStore } from '../state/ui.ts';

/** How long an error card stays up, unless the pointer rests on it. */
const SHOWN_MS = 10_000;

/**
 * Logs an error for the Errors page in Settings and pops up a card naming it. The card's Details button opens that
 * page, where the full detail waits.
 */
export function showError(entry: Omit<ErrorEntry, 'id' | 'at'>): ErrorEntry {
  const kept = errorsStore.getState().report(entry);
  toast.error(entry.text, {
    id: `error-${kept.id}`,
    description: entry.where,
    duration: SHOWN_MS,
    testId: 'error-card',
    action: { label: 'Details', onClick: () => uiStore.getState().openSettings('errors') },
  });
  return kept;
}
