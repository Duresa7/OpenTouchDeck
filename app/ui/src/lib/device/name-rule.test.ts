import { describe, expect, test } from 'vitest';
import vectors from '../../../../../schema/protocol/name-vectors-v2.json';
import { nameProblem, SPECIAL_CHARACTERS } from './settings.ts';
import { nameError } from './onboarding.ts';
import { hasHiddenCharacter } from './name-rule.ts';

const codePoints = (label: string): number[] => {
  const [first, last = first] = label.split('-').map((part) => parseInt(part, 16));
  return Array.from({ length: last - first + 1 }, (_, i) => first + i);
};

describe('the shared name vectors', () => {
  test.each(vectors.accepted.map((v) => [v.label, v.name]))('%s is accepted', (_label, name) => {
    expect(nameProblem(name)).toBeNull();
    expect(nameError(name)).toBeNull();
  });

  test.each(vectors.rejected.map((v) => [v.label, v.name]))('%s is refused', (_label, name) => {
    expect(nameProblem(name)).toBe(SPECIAL_CHARACTERS);
    expect(nameError(name)).toBe(SPECIAL_CHARACTERS);
  });

  test('every character of the pinned format table and each Hangul filler is refused, and no other is', () => {
    const hidden = new Set([...vectors.format_ranges.flatMap(codePoints), ...vectors.hangul_fillers.map((h) => parseInt(h, 16))]);
    for (let code = 0x20; code <= 0x10ffff; code++) {
      if (code === 0x7f || (code >= 0xd800 && code <= 0xdfff)) continue;
      expect(hasHiddenCharacter(`a${String.fromCodePoint(code)}b`), `U+${code.toString(16)}`).toBe(hidden.has(code));
    }
  });
});
