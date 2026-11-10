// The characters a Device name refuses: controls (Cc, U+0000 to U+001F and U+007F
// to U+009F), format characters (General_Category Cf, pinned to Unicode 17.0) and
// the Hangul fillers. They are invisible or reorder text, so a name could look like
// another name. The format table is
// the one link-v2.md and net-v2.md pin, so no runtime's Unicode version changes it;
// schema/protocol/name-vectors-v2.json checks it.
const FORMAT_RANGES: readonly (readonly [number, number])[] = [
  [0x00ad, 0x00ad], [0x0600, 0x0605], [0x061c, 0x061c], [0x06dd, 0x06dd],
  [0x070f, 0x070f], [0x0890, 0x0891], [0x08e2, 0x08e2], [0x180e, 0x180e],
  [0x200b, 0x200f], [0x202a, 0x202e], [0x2060, 0x2064], [0x2066, 0x206f],
  [0xfeff, 0xfeff], [0xfff9, 0xfffb], [0x110bd, 0x110bd], [0x110cd, 0x110cd],
  [0x13430, 0x1343f], [0x1bca0, 0x1bca3], [0x1d173, 0x1d17a], [0xe0001, 0xe0001],
  [0xe0020, 0xe007f],
];

const HANGUL_FILLERS: readonly number[] = [0x115f, 0x1160, 0x3164, 0xffa0];

/** True when the name holds a control, a format character or a Hangul filler. */
export function hasHiddenCharacter(name: string): boolean {
  for (const character of name) {
    const code = character.codePointAt(0)!;
    if (code <= 0x1f || (code >= 0x7f && code <= 0x9f) || HANGUL_FILLERS.includes(code)) return true;
    if (FORMAT_RANGES.some(([first, last]) => code >= first && code <= last)) return true;
  }
  return false;
}
