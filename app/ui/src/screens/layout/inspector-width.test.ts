import { describe, expect, it } from 'vitest';
import { clampWidth, inspectorLimits, keyWidth, readWidth, writeWidth } from './inspector-width.ts';

function memory() {
  const items = new Map<string, string>();
  return { getItem: (key: string) => items.get(key) ?? null, setItem: (key: string, value: string) => void items.set(key, value) };
}

describe('the details panel width', () => {
  it('keeps the Device at least 460px of canvas and the panel between 340px and 600px', () => {
    expect(inspectorLimits(1280)).toEqual({ min: 340, max: 600, fixed: false });
    expect(inspectorLimits(1000)).toEqual({ min: 340, max: 540, fixed: false });
    expect(inspectorLimits(800)).toEqual({ min: 340, max: 340, fixed: false });
  });

  it('falls back to a fixed 340px panel below 800px, so the minimum never passes the maximum', () => {
    expect(inspectorLimits(799)).toEqual({ min: 340, max: 340, fixed: true });
    expect(inspectorLimits(640)).toEqual({ min: 340, max: 340, fixed: true });
  });

  it('clamps a width to the limits', () => {
    const limits = inspectorLimits(1000);
    expect(clampWidth(400, limits)).toBe(400);
    expect(clampWidth(200, limits)).toBe(340);
    expect(clampWidth(900, limits)).toBe(540);
    expect(clampWidth(412.6, limits)).toBe(413);
  });

  it('moves with the arrow keys, Shift for bigger steps, and jumps to the limits with Home and End', () => {
    const limits = inspectorLimits(1280);
    expect(keyWidth(400, 'ArrowLeft', false, limits)).toBe(416);
    expect(keyWidth(400, 'ArrowRight', false, limits)).toBe(384);
    expect(keyWidth(400, 'ArrowLeft', true, limits)).toBe(464);
    expect(keyWidth(400, 'ArrowRight', true, limits)).toBe(340);
    expect(keyWidth(590, 'ArrowLeft', false, limits)).toBe(600);
    expect(keyWidth(400, 'Home', false, limits)).toBe(340);
    expect(keyWidth(400, 'End', false, limits)).toBe(600);
    expect(keyWidth(400, 'Enter', false, limits)).toBeNull();
  });

  it('remembers the width chosen, and starts at 400px', () => {
    const storage = memory();
    expect(readWidth(storage)).toBe(400);
    writeWidth(472.4, storage);
    expect(readWidth(storage)).toBe(472);
    expect(readWidth(null)).toBe(400);
    expect(readWidth({ getItem: () => 'wide', setItem: () => {} })).toBe(400);
    expect(readWidth({ getItem: () => { throw new Error('blocked'); }, setItem: () => {} })).toBe(400);
  });
});
