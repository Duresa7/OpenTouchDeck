import { useCompanion } from '../../lib/companion/connection.ts';
import { usePackThumbnail } from '../../lib/icon-packs/hooks.ts';
import { monogram } from '../../lib/icons/catalog.ts';

/** One Icon pack icon, drawn from its same-origin SVG. A letter stands in if it can't load. */
export function PackThumbnail({ prefix, name, size }: { prefix: string; name: string; size: number }) {
  const companion = useCompanion();
  const thumbnail = usePackThumbnail(companion, prefix, name);
  if (thumbnail === null) return <span className="pack-thumb is-loading" style={{ width: size, height: size }} aria-hidden />;
  if (thumbnail.kind === 'missing')
    return (
      <span className="icon-cell-mark" aria-hidden>
        {monogram(name)}
      </span>
    );
  return <img className="pack-thumb" src={thumbnail.url} width={size} height={size} alt="" draggable={false} data-testid="pack-thumb" />;
}
