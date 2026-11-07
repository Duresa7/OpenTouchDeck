import { useEffect, useId, useRef } from 'react';
import { Dialog } from '@base-ui/react/dialog';
import { ChevronLeft, FileUp, X } from 'lucide-react';
import type { PackInfo } from '../../lib/companion/api.ts';
import { useCompanion } from '../../lib/companion/connection.ts';
import { iconPacksOf, useIconPacks, useInstallStage } from '../../lib/icon-packs/hooks.ts';
import { packFacts, stageText } from '../../lib/icon-packs/model.ts';
import type { PackWork } from '../../lib/icon-packs/store.ts';
import { PackThumbnail } from '../../screens/layout/PackThumbnail.tsx';
import { button } from '../button.ts';

const SAMPLES = 4;

function Status({ work }: { work: Extract<PackWork, { kind: 'installing' }> }) {
  const stage = useInstallStage(work.job);
  return (
    <span className="pack-status" role="status" data-testid="pack-status" data-stage={stage ?? 'download'}>
      {stageText(stage)}
    </span>
  );
}

/** A problem in the copy guide's words, with the Companion's own message under Details. */
function Problem({ className, testId, text, detail }: { className: string; testId: string; text: string; detail: string }) {
  return (
    <div className={className} role="alert">
      <p data-testid={testId}>{text}</p>
      {detail && detail !== text && (
        <details className="problem-details" data-testid={`${testId}-details`}>
          <summary>Details</summary>
          <p>{detail}</p>
        </details>
      )}
    </div>
  );
}

function PackRow({ pack, work, onInstall, onRemove }: { pack: PackInfo; work: PackWork | undefined; onInstall(): void; onRemove(): void }) {
  const busy = work?.kind === 'installing' || work?.kind === 'removing';
  return (
    <div className="row pack-row" data-testid="icon-pack" data-prefix={pack.prefix} data-installed={pack.installed} data-state={work?.kind ?? 'idle'}>
      <div className="row-text">
        <span className="row-title">{pack.name}</span>
        <span className="row-sub tabular">
          {packFacts(pack)} ·{' '}
          {pack.license.url ? (
            <a href={pack.license.url} target="_blank" rel="noreferrer" data-testid="pack-license">
              {pack.license.title}
            </a>
          ) : (
            <span data-testid="pack-license">{pack.license.title}</span>
          )}
          {pack.attribution && ' · Credit required'}
          {pack.brands && ' · Logos'}
        </span>
        {pack.installed && pack.samples.length > 0 && (
          <span className="pack-samples" aria-hidden>
            {pack.samples.slice(0, SAMPLES).map((name) => (
              <span className="pack-sample" key={name}>
                <PackThumbnail prefix={pack.prefix} name={name} size={20} />
              </span>
            ))}
          </span>
        )}
        {work?.kind === 'failed' && <Problem className="row-problem" testId="pack-problem" text={work.text} detail={work.detail} />}
      </div>
      {work?.kind === 'installing' ? (
        <Status work={work} />
      ) : pack.installed ? (
        <button type="button" className={button()} aria-disabled={busy || undefined} aria-label={`Remove ${pack.name}`} onClick={() => !busy && onRemove()} data-testid="pack-remove">
          {work?.kind === 'removing' ? 'Removing…' : 'Remove'}
        </button>
      ) : (
        <button type="button" className={button()} aria-label={`Download ${pack.name}`} onClick={onInstall} data-testid="pack-install">
          {work?.kind === 'failed' ? 'Retry' : 'Download'}
        </button>
      )}
    </div>
  );
}

export function IconPacksView({ onBack, focusBack }: { onBack(): void; focusBack: boolean }) {
  const companion = useCompanion();
  const packs = iconPacksOf(companion);
  const list = useIconPacks(companion, (s) => s.packs);
  const load = useIconPacks(companion, (s) => s.load);
  const loadProblem = useIconPacks(companion, (s) => s.loadProblem);
  const work = useIconPacks(companion, (s) => s.work);
  const importing = useIconPacks(companion, (s) => s.importing);
  const importProblem = useIconPacks(companion, (s) => s.importProblem);
  const back = useRef<HTMLButtonElement>(null);
  const file = useRef<HTMLInputElement>(null);
  const ids = useId();
  useEffect(() => {
    if (focusBack) back.current?.focus();
  }, [focusBack]);

  const curated = list?.filter((p) => p.origin === 'curated') ?? [];
  const mine = list?.filter((p) => p.origin === 'imported') ?? [];
  const row = (pack: PackInfo) => (
    <PackRow key={pack.prefix} pack={pack} work={work[pack.prefix]} onInstall={() => void packs.install(pack.prefix)} onRemove={() => void packs.remove(pack.prefix)} />
  );

  return (
    <>
      <div className="dialog-head">
        <div className="dialog-head-start">
          <button ref={back} type="button" className={button({ kind: 'icon' })} aria-label="Back" onClick={onBack}>
            <ChevronLeft size={18} strokeWidth={1.9} aria-hidden />
          </button>
          <Dialog.Title className="dialog-title">Icon packs</Dialog.Title>
        </div>
        <Dialog.Close className={button({ kind: 'icon' })} aria-label="Close">
          <X size={18} strokeWidth={1.9} aria-hidden />
        </Dialog.Close>
      </div>
      <p className="dialog-lede">Download Icon packs to search offline in the icon picker, or import your own. Nothing is installed until you choose.</p>

      <section className="settings-section" aria-labelledby={`${ids}-curated`} data-testid="icon-packs" data-state={load}>
        <h3 id={`${ids}-curated`}>Packs</h3>
        <div className="group">
          {list === null ? (
            <div className="row">
              <span className="row-note">{load === 'failed' ? loadProblem : 'Loading…'}</span>
              {load === 'failed' && (
                <button type="button" className={button()} onClick={() => void packs.load()}>
                  Retry
                </button>
              )}
            </div>
          ) : (
            curated.map(row)
          )}
        </div>
      </section>

      <section className="settings-section" aria-labelledby={`${ids}-mine`}>
        <div className="settings-section-head">
          <h3 id={`${ids}-mine`}>Your packs</h3>
          <button type="button" className={button({ size: 'small' })} aria-disabled={importing !== null || undefined} onClick={() => importing === null && file.current?.click()} data-testid="pack-import">
            <FileUp size={14} strokeWidth={1.9} aria-hidden />
            {importing === null ? 'Import a pack file' : 'Importing…'}
          </button>
        </div>
        <div className="group">
          {mine.length ? (
            mine.map(row)
          ) : (
            <div className="row">
              <span className="row-note">Import an Iconify JSON file to search its icons here. Its license is yours to check.</span>
            </div>
          )}
        </div>
        {importProblem && <Problem className="settings-problem" testId="pack-import-problem" text={importProblem.text} detail={importProblem.detail} />}
        <input
          ref={file}
          type="file"
          accept=".json,application/json"
          hidden
          onChange={(event) => {
            const chosen = event.target.files?.[0];
            event.target.value = '';
            if (chosen) void packs.importFile(chosen, chosen.name);
          }}
          data-testid="pack-import-file"
        />
      </section>

      <p className="settings-footnote">Downloads come from the npm registry and must match a pinned checksum. Logos belong to their owners.</p>
    </>
  );
}
