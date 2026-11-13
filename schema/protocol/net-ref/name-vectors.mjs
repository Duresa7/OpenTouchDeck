// Regenerates schema/protocol/name-vectors-v2.json, the names that link-v2 and
// net-v2 share: a link-v2 Device name and a net-v2 `companion_name` both refuse
// every control character (Cc), every format character (Cf, pinned to Unicode
// 17.0) and the Hangul fillers.
// Run from the repository root:
//   node schema/protocol/net-ref/name-vectors.mjs --write
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { FORMAT_RANGES, HANGUL_FILLERS } from './net-v2.mjs';

// The control characters (General_Category Cc): C0, then DEL and the C1 controls.
// Unicode's stability policy fixes this set.
export const CONTROL_RANGES = [[0x0000, 0x001f], [0x007f, 0x009f]];

const label = (codePoint) => `U+${codePoint.toString(16).toUpperCase().padStart(4, '0')}`;
const hex = (codePoint) => codePoint.toString(16).padStart(4, '0');
const rangeLabel = ([first, last]) => (first === last ? hex(first) : `${hex(first)}-${hex(last)}`);
const text = (codePoint) => String.fromCodePoint(codePoint);

// The characters next to a range that stay allowed: letters, marks, numbers,
// punctuation and symbols that a table off by one would refuse.
function neighbours() {
  const found = new Set();
  const add = (codePoint) => {
    if (/[\p{L}\p{M}\p{N}\p{P}\p{S}]/u.test(text(codePoint)) && !HANGUL_FILLERS.includes(codePoint)) found.add(codePoint);
  };
  for (const [first, last] of FORMAT_RANGES) {
    add(first - 1);
    add(last + 1);
  }
  for (const filler of HANGUL_FILLERS) {
    add(filler - 1);
    add(filler + 1);
  }
  return [...found].sort((a, b) => a - b);
}

const SAMPLES = [
  0x0080, 0x0085, 0x009f,
  0x00ad, 0x061c, 0x0600, 0x06dd, 0x115f, 0x1160, 0x180e, 0x200b, 0x200c, 0x200d, 0x200e, 0x200f,
  0x202a, 0x202b, 0x202c, 0x202d, 0x202e, 0x2060, 0x2063, 0x2066, 0x2067, 0x2068, 0x2069, 0x206f,
  0x3164, 0xfeff, 0xffa0, 0xfff9, 0xfffb, 0x110bd, 0x13430, 0x1d173, 0xe0001, 0xe0041, 0xe007f,
];

export function buildNameVectors() {
  return {
    protocol: 'touchdeck/names-v2',
    notice: 'Shared by link-v2 Device names and net-v2 companion names. A name that holds a control character, a format character or a Hangul filler is refused; the rest of each rule (length, blank names) is in its own spec.',
    control_ranges: CONTROL_RANGES.map(rangeLabel),
    format_ranges: FORMAT_RANGES.map(rangeLabel),
    hangul_fillers: HANGUL_FILLERS.map(hex),
    accepted: [
      { label: 'ascii', name: 'Desk PC' },
      { label: 'accented', name: 'Bureau d’été' },
      { label: 'right-to-left letters', name: 'מחשב' },
      { label: 'emoji with variation selector', name: 'Desk \u{1f5a5}️' },
      { label: 'Hangul letters', name: '한글' },
      { label: 'unassigned neighbour U+2065', name: 'Desk⁥PC' },
      { label: 'U+007E before DEL and the C1 controls', name: 'Desk~PC' },
      { label: 'no-break space U+00A0 after the C1 controls', name: 'Desk\u00a0PC' },
      ...neighbours().map((codePoint) => ({ label: `${label(codePoint)} beside a range`, name: `Desk${text(codePoint)}PC` })),
    ],
    rejected: [
      ...SAMPLES.map((codePoint) => ({ label: label(codePoint), name: `Desk${text(codePoint)}PC` })),
      { label: 'leading U+FEFF', name: '﻿Desk' },
      { label: 'trailing U+200E', name: 'Desk‎' },
      { label: 'only U+3164', name: 'ㅤ' },
      { label: 'only U+FFA0', name: 'ﾠ' },
      { label: 'U+115F then U+1160', name: 'ᅟᅠ' },
      { label: 'letters around U+200D', name: 'a‍b' },
    ],
  };
}

// Every non-ASCII character is written as a unicode escape, so no invisible
// character sits in the file.
export const serialize = (vectors) => `${JSON.stringify(vectors, null, 2)
  .replace(/[\u007f-￿]/g, (c) => `\\u${c.charCodeAt(0).toString(16).padStart(4, '0')}`)}\n`;

export const namesPath = new URL('../name-vectors-v2.json', import.meta.url);
export const readNameVectors = () => JSON.parse(readFileSync(namesPath, 'utf8'));

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv[2] !== '--write') throw new Error('Use --write to regenerate the shared name vectors');
  writeFileSync(namesPath, serialize(buildNameVectors()));
  process.stdout.write('Wrote name-vectors-v2.json.\n');
}
