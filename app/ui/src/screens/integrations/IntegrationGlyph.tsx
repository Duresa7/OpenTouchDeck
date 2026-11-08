import type { ComponentType } from 'react';
import { Activity, Braces, Building2, Cpu, Gauge, Monitor, Music2, Plug, Server, Shapes, Volume2, Wifi, type LucideProps } from 'lucide-react';

const GLYPHS: Partial<Record<string, ComponentType<LucideProps>>> = {
  activity: Activity,
  braces: Braces,
  code: Braces,
  cpu: Cpu,
  gauge: Gauge,
  monitor: Monitor,
  music: Music2,
  server: Server,
  volume: Volume2,
  wifi: Wifi,
  shapes: Shapes,
  logos: Building2,
};

/** An Integration's glyph from the icon name it reports. */
export function IntegrationGlyph({ icon, size = 20 }: { icon: string; size?: number }) {
  const Glyph = GLYPHS[icon] ?? Plug;
  return (
    <span className="integration-glyph" aria-hidden>
      <Glyph size={size} strokeWidth={1.9} />
    </span>
  );
}
