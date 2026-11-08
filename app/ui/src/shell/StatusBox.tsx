import clsx from 'clsx';

/** Green when it works, red when it doesn't, amber while it's on its way: connecting, checking, updating. */
export type Tone = 'ok' | 'bad' | 'wait';

export interface StatusBoxProps {
  tone: Tone;
  /** The status in words. Shown beside the box with `showWords`; otherwise it's the box's accessible name. */
  label: string;
  showWords?: boolean;
  className?: string;
  testId?: string;
}

/**
 * A small square that says how something is doing: green, red with a cut so it doesn't rely on colour, or amber. Only
 * its colour changes, over 150ms; nothing animates when it appears.
 */
export function StatusBox({ tone, label, showWords = false, className, testId }: StatusBoxProps) {
  if (!showWords) {
    return <span className={clsx('status-box', className)} role="img" aria-label={label} data-tone={tone} data-testid={testId} />;
  }
  return (
    <span className={clsx('status', className)} data-tone={tone} data-testid={testId}>
      <i className="status-box" data-tone={tone} aria-hidden />
      <span className="status-words">{label}</span>
    </span>
  );
}
