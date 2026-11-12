/** The details panel's width: roomy by default, never narrower than its fields need, never squeezing the Device. */
export const INSPECTOR_DEFAULT = 400;
export const INSPECTOR_MIN = 340;
export const INSPECTOR_MAX = 600;
/** What the canvas keeps beside the panel: the smallest drawn Device plus its padding. */
export const CANVAS_MIN = 460;
/** The Pages column's width, between the nav rail and the canvas. */
export const PAGES_W = 160;

const STORAGE_KEY = 'touchdeck.inspector-width';

export interface InspectorLimits {
  min: number;
  max: number;
  /** Too narrow for both: the panel stays at its minimum, without a resize handle, and the canvas takes what's left. */
  fixed: boolean;
}

/** The widths the panel may take in an editor this wide. */
export function inspectorLimits(editorWidth: number): InspectorLimits {
  if (editorWidth < INSPECTOR_MIN + CANVAS_MIN) return { min: INSPECTOR_MIN, max: INSPECTOR_MIN, fixed: true };
  return { min: INSPECTOR_MIN, max: Math.min(INSPECTOR_MAX, editorWidth - CANVAS_MIN), fixed: false };
}

/**
 * Whether the editor is too narrow for the Pages column, the canvas at its minimum and the details panel side by
 * side. Then one panel is out at a time: the details panel while it's out, the Pages column otherwise.
 */
export function onePanel(editorWidth: number): boolean {
  return editorWidth < PAGES_W + INSPECTOR_MIN + CANVAS_MIN;
}

export function clampWidth(width: number, limits: Pick<InspectorLimits, 'min' | 'max'>): number {
  return Math.round(Math.min(limits.max, Math.max(limits.min, width)));
}

/**
 * Where a key moves the handle on the panel's left edge: left widens the panel and right narrows it, by 16px or 64px
 * with Shift; Home and End jump to the limits. Null for any other key.
 */
export function keyWidth(width: number, key: string, shift: boolean, limits: Pick<InspectorLimits, 'min' | 'max'>): number | null {
  const step = shift ? 64 : 16;
  switch (key) {
    case 'ArrowLeft':
      return clampWidth(width + step, limits);
    case 'ArrowRight':
      return clampWidth(width - step, limits);
    case 'Home':
      return limits.min;
    case 'End':
      return limits.max;
    default:
      return null;
  }
}

type Prefs = Pick<Storage, 'getItem' | 'setItem'>;

function localStorageOrNull(): Prefs | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/** The width chosen last, or the default. */
export function readWidth(storage: Prefs | null = localStorageOrNull()): number {
  try {
    const value = Number(storage?.getItem(STORAGE_KEY));
    return Number.isFinite(value) && value > 0 ? value : INSPECTOR_DEFAULT;
  } catch {
    return INSPECTOR_DEFAULT;
  }
}

export function writeWidth(width: number, storage: Prefs | null = localStorageOrNull()) {
  try {
    storage?.setItem(STORAGE_KEY, String(Math.round(width)));
  } catch {
    // Without storage the width lasts until the window closes.
  }
}
