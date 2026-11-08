import type { IntegrationSummary, PresetTile } from '../../lib/companion/api.ts';
import { plural } from '../../lib/layout/messages.ts';
import { lineOf, statusOf } from '../../lib/integrations/model.ts';
import { button } from '../../shell/button.ts';
import { usePresetNeeds } from './addToLayout.ts';
import { IntegrationGlyph, PresetButton } from './PresetPreview.tsx';

const ROW_PRESETS = 3;

function Row({ integration, onOpen, onAdd }: { integration: IntegrationSummary; onOpen(id: string, setUp: boolean): void; onAdd(preset: PresetTile): void }) {
  const status = statusOf(integration);
  const needs = usePresetNeeds();
  const shown = integration.presets.slice(0, ROW_PRESETS);
  const rest = integration.presets.length - shown.length;
  return (
    <li
      className="integration"
      data-testid="integration"
      data-id={integration.id}
      data-state={integration.status.state}
      data-delivery={integration.status.delivery.length}
    >
      <button type="button" className="integration-open" onClick={() => onOpen(integration.id, false)} data-testid="integration-open">
        <IntegrationGlyph icon={integration.icon} />
        <span className="integration-text">
          <span className="integration-name">{integration.name}</span>
          <span className="integration-about">{lineOf(integration.id)}</span>
        </span>
      </button>
      <span className="integration-presets">
        {shown.map((preset) => (
          <PresetButton key={preset.id} preset={preset} streams={integration.streams} onAdd={onAdd} needs={needs(preset)} />
        ))}
        {rest > 0 && (
          <button
            type="button"
            className="integration-more"
            aria-label={`Show ${plural(integration.presets.length, 'Preset tile')}`}
            onClick={() => onOpen(integration.id, false)}
            data-testid="integration-more"
          >
            +{rest}
          </button>
        )}
      </span>
      <span className="integration-state">
        {status.needsSetup ? (
          <button type="button" className={button()} onClick={() => onOpen(integration.id, true)} data-testid="integration-setup">
            Set up
          </button>
        ) : (
          <span className="status" data-testid="integration-status" data-live={status.live}>
            {status.tone && <i className="status-box" data-tone={status.tone} aria-hidden />}
            {status.words}
          </span>
        )}
      </span>
    </li>
  );
}

/** The list sheet: every Integration with its state on the right. */
export function IntegrationList({ integrations, onOpen, onAdd }: { integrations: readonly IntegrationSummary[]; onOpen(id: string, setUp: boolean): void; onAdd(preset: PresetTile): void }) {
  return (
    <ul className="integration-list" aria-label="Integrations" data-testid="integration-list">
      {integrations.map((integration) => (
        <Row key={integration.id} integration={integration} onOpen={onOpen} onAdd={onAdd} />
      ))}
    </ul>
  );
}
