import { describe, expect, test } from 'vitest';
import type { PackInfo } from '../../companion/api.ts';
import { ApiError } from '../../companion/errors.ts';
import { credits, formatBytes, importProblem, installProblem, isPackHit, packFacts, packPrefixOf, sourceName, stageText } from '../model.ts';

const pack = (change: Partial<PackInfo> = {}): PackInfo => ({
  prefix: 'fa6-brands',
  name: 'Font Awesome 6 Brands',
  origin: 'curated',
  installed: false,
  version: '1.2.6',
  download_bytes: 1_234_567,
  size_bytes: null,
  icons: 2064,
  license: { title: 'CC BY 4.0', spdx: 'CC-BY-4.0', url: 'https://creativecommons.org/licenses/by/4.0/' },
  author: { name: 'Dave Gandy', url: 'https://fontawesome.com/' },
  attribution: true,
  brands: true,
  palette: false,
  samples: ['spotify'],
  ...change,
});

const error = (code: ConstructorParameters<typeof ApiError>[0]) => new ApiError(code, code);

describe('pack hits', () => {
  test('carry their pack prefix in the source and the pack name in the category', () => {
    const hit = { source: 'iconify:mdi', category: 'Material Design Icons' };
    expect(isPackHit(hit)).toBe(true);
    expect(packPrefixOf(hit.source)).toBe('mdi');
    expect(sourceName(hit, { lucide: 'Lucide' })).toBe('Material Design Icons');
    expect(sourceName({ source: 'iconify:mdi', category: null }, {})).toBe('mdi');
  });

  test('leave built-in catalogs to their own names', () => {
    expect(isPackHit({ source: 'lucide' })).toBe(false);
    expect(packPrefixOf('lucide')).toBeNull();
    expect(packPrefixOf('iconify:')).toBeNull();
    expect(sourceName({ source: 'lucide', category: 'Media' }, { lucide: 'Lucide' })).toBe('Lucide');
  });
});

describe('pack facts', () => {
  test('show the download size before install and the size on disk after', () => {
    expect(packFacts(pack())).toBe('2,064 icons · 1.2 MB');
    expect(packFacts(pack({ installed: true, size_bytes: 25 * 1024 * 1024 }))).toBe('2,064 icons · 25 MB');
    expect(packFacts(pack({ icons: 1, download_bytes: null }))).toBe('1 icon');
  });

  test('round bytes to whole KB and one decimal under 10 MB', () => {
    expect([formatBytes(512), formatBytes(1024), formatBytes(10), formatBytes(1536 * 1024)]).toEqual(['512 B', '1 KB', '10 B', '1.5 MB']);
  });
});

describe('credits', () => {
  test('name installed packs whose license asks for credit', () => {
    const list = [pack({ installed: true }), pack({ prefix: 'mdi', attribution: false, installed: true }), pack({ prefix: 'other' })];
    expect(credits(list)).toEqual([{ key: 'fa6-brands', name: 'Font Awesome 6 Brands', url: 'https://creativecommons.org/licenses/by/4.0/', note: 'by Dave Gandy (CC BY 4.0, modified)' }]);
  });

  test('name Brandfetch last while its search is on', () => {
    const list = [pack({ installed: true })];
    expect(credits(list, true).map((credit) => [credit.key, credit.name, credit.note])).toEqual([
      ['fa6-brands', 'Font Awesome 6 Brands', 'by Dave Gandy (CC BY 4.0, modified)'],
      ['brandfetch', 'Brandfetch', '(your keys)'],
    ]);
    expect(credits([], true)).toEqual([{ key: 'brandfetch', name: 'Brandfetch', url: 'https://brandfetch.com', note: '(your keys)' }]);
  });
});

describe('copy', () => {
  test('names each install stage', () => {
    expect([null, 'download', 'verify', 'extract', 'done'].map((s) => stageText(s as never))).toEqual(['Downloading…', 'Downloading…', 'Checking…', 'Unpacking…', 'Finishing…']);
  });

  test('explains a failed install and import in the user’s terms', () => {
    expect(installProblem(error('icon_pack.integrity'), 'Lucide')).toBe("Lucide didn't match its checksum, so it wasn't installed. Retry later.");
    expect(installProblem(error('icon_pack.download_failed'), 'Lucide')).toContain('Check the internet connection');
    expect(importProblem(error('icon_pack.exists'), 'mine.json')).toContain('same prefix');
    expect(importProblem(error('icon_pack.invalid'), 'mine.json')).toContain("isn't an Iconify JSON pack");
    expect(importProblem(error('icon_pack.too_many'), 'mine.json')).toBe('You can import up to 20 packs. Remove one, then import “mine.json” again.');
  });

  test('pick the fix by what went wrong, not one guess for every failure', () => {
    const texts = ['icon_pack.download_failed', 'icon_pack.invalid', 'internal', 'icon_pack.not_found'].map((code) => installProblem(error(code), 'Lucide'));
    expect(texts).toEqual([
      "Couldn't download Lucide. Check the internet connection, then retry.",
      "Lucide couldn't be unpacked, so it wasn't installed. Update Open TouchDeck, then retry.",
      "Couldn't save Lucide on this PC. Check that the disk has free space, then retry.",
      "Couldn't install Lucide. Restart Open TouchDeck, then retry.",
    ]);
  });
});
