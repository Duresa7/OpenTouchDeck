import { describe, expect, it } from 'vitest';
import { createIconSources, sourceOn } from './icon-sources.ts';

function memory() {
  const items = new Map<string, string>();
  return { items, getItem: (key: string) => items.get(key) ?? null, setItem: (key: string, value: string) => void items.set(key, value) };
}

describe('icon source switches', () => {
  it('apply a default once, so a deliberate off survives a restart', () => {
    const storage = memory();
    const first = createIconSources(storage);
    expect(first.getState().iconPacks).toBeNull();
    first.getState().settle('iconPacks', true);
    expect(first.getState().iconPacks).toBe(true);
    first.getState().set('iconPacks', false);
    first.getState().settle('iconPacks', true);
    expect(first.getState().iconPacks).toBe(false);

    const restarted = createIconSources(storage);
    restarted.getState().settle('iconPacks', true);
    expect(restarted.getState().iconPacks).toBe(false);
    expect(restarted.getState().brandfetch).toBeNull();
  });

  it('work without storage', () => {
    const sources = createIconSources(null);
    sources.getState().set('brandfetch', true);
    expect(sources.getState().brandfetch).toBe(true);
  });

  it('follow what is set up until someone sets them', () => {
    expect(sourceOn(null, true)).toBe(true);
    expect(sourceOn(null, false)).toBe(false);
    expect(sourceOn(false, true)).toBe(false);
    expect(sourceOn(true, false)).toBe(true);
  });
});
