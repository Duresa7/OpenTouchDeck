import { useState } from 'react';
import clsx from 'clsx';
import { badgeOf, countsLine, formatCount, resetLine, type Usage, type UsageWindow } from '../../lib/integrations/model.ts';
import { button } from '../../shell/button.ts';
import { StatusBox } from '../../shell/StatusBox.tsx';

function Limit({ window, testId }: { window: UsageWindow | null; testId: string }) {
  if (!window) {
    return (
      <td data-testid={testId}>
        <span className="usage-empty">—</span>
      </td>
    );
  }
  const share = Math.min(100, Math.max(0, window.pct));
  return (
    <td data-testid={testId}>
      <span className="usage-pct">{formatCount(window.pct)}%</span>
      <span className="meter" aria-hidden>
        <span style={{ transform: `scaleX(${share / 100})` }} />
      </span>
      <span className="usage-sub">{resetLine(window)}</span>
    </td>
  );
}

/** Every account the hub reads, with the share of each limit used: what the AI usage page showed. */
export function UsageTable({ usage }: { usage: Usage }) {
  const [copied, setCopied] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const copy = async (account: string) => {
    try {
      await navigator.clipboard.writeText(account);
      setCopied(account);
      setProblem(null);
    } catch {
      setProblem("Couldn't copy. Select the account in the table and copy it.");
    }
  };
  return (
    <>
      <div className="usage-scroll">
        <table className="usage-table" data-testid="usage-accounts">
          <thead>
            <tr>
              <th scope="col">Account</th>
              <th scope="col">Service</th>
              <th scope="col">Plan</th>
              <th scope="col">5-hour limit</th>
              <th scope="col">7-day limit</th>
              <th scope="col">Status</th>
            </tr>
          </thead>
          <tbody>
            {usage.accounts.map((account, index) => (
              <tr key={`${account.key}:${index}`} className={clsx(usage.stale && 'is-stale')} data-testid="usage-account" data-account={account.key}>
                <td className="usage-account">
                  <code>{account.key}</code>
                  <button type="button" className={clsx(button({ size: 'small' }), 'usage-copy')} aria-label={`Copy ${account.key}`} onClick={() => void copy(account.key)} data-testid="usage-copy">
                    {copied === account.key ? 'Copied' : 'Copy'}
                  </button>
                </td>
                <td>{account.provider}</td>
                <td data-testid="usage-plan">{account.plan ?? '—'}</td>
                <Limit window={account.five_hour} testId="usage-five-hour" />
                <Limit window={account.seven_day} testId="usage-seven-day" />
                <td data-testid="usage-status">
                  <StatusBox tone={account.status.toLowerCase() === 'active' ? 'ok' : 'bad'} label={badgeOf(account.status)} showWords />
                  {account.usage_error && <span className="usage-sub is-error">{account.usage_error}</span>}
                  <span className="usage-sub tabular">{countsLine(account)}</span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {problem && (
        <p className="field-error" role="alert">
          {problem}
        </p>
      )}
      <p className="field-hint">Limits show the share used. Copy an account to show only that account on a Tile. Gemini and some other services report request counts only.</p>
    </>
  );
}
