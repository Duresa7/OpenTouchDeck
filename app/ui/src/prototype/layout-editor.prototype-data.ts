// PROTOTYPE — throwaway, answers: how should the Layout editor lay out so the nav never shifts?
// Static mock data only, so the prototype never depends on the real editor while it changes.

export type IconKey =
  | 'play'
  | 'back'
  | 'next'
  | 'mute'
  | 'volUp'
  | 'volDown'
  | 'music'
  | 'mic'
  | 'copy'
  | 'paste'
  | 'camera'
  | 'lock'
  | 'terminal'
  | 'globe'
  | 'calc'
  | 'folder'
  | 'cpu'
  | 'memory'
  | 'gauge'
  | 'wifi'
  | 'disk'
  | 'temp'
  | 'lamp'
  | 'bulb'
  | 'sun'
  | 'moon'
  | 'power';

export interface Tile {
  id: string;
  /** Row-major cell index in the Page grid. */
  cell: number;
  label: string;
  icon: IconKey;
  action: string;
  /** A live reading shown big on the Tile, for stats. */
  value?: string;
}

export interface Page {
  id: string;
  title: string;
  rows: number;
  cols: number;
  tiles: Tile[];
}

export const ACTIONS = [
  'Media: Play or pause',
  'Media: Previous track',
  'Media: Next track',
  'Media: Mute',
  'Media: Volume up',
  'Media: Volume down',
  'Keys: Ctrl+C',
  'Keys: Ctrl+V',
  'Keys: Win+Shift+S',
  'Keys: Win+L',
  'Open app: Terminal',
  'Open app: Browser',
  'Open app: Calculator',
  'Open folder: Documents',
  'Show reading: This PC · CPU',
  'Show reading: This PC · Memory used',
  'Show reading: This PC · GPU',
  'Show reading: This PC · Network send',
  'Show reading: This PC · Disk used',
  'Show reading: This PC · CPU temperature',
  'Home Assistant: Toggle light',
  'Home Assistant: Run scene',
] as const;

let next = 100;
export const newId = (prefix: string) => `${prefix}-${next++}`;

export const DEVICE_NAME = 'Desk Deck';

export const PAGES: Page[] = [
  {
    id: 'media',
    title: 'Media',
    rows: 3,
    cols: 4,
    tiles: [
      { id: 't1', cell: 0, label: 'Previous', icon: 'back', action: 'Media: Previous track' },
      { id: 't2', cell: 1, label: 'Play', icon: 'play', action: 'Media: Play or pause' },
      { id: 't3', cell: 2, label: 'Next', icon: 'next', action: 'Media: Next track' },
      { id: 't4', cell: 3, label: 'Mute', icon: 'mute', action: 'Media: Mute' },
      { id: 't5', cell: 4, label: 'Volume down', icon: 'volDown', action: 'Media: Volume down' },
      { id: 't6', cell: 5, label: 'Volume up', icon: 'volUp', action: 'Media: Volume up' },
      { id: 't7', cell: 8, label: 'Spotify', icon: 'music', action: 'Open app: Browser' },
      { id: 't8', cell: 9, label: 'Mic', icon: 'mic', action: 'Keys: Win+Shift+S' },
    ],
  },
  {
    id: 'shortcuts',
    title: 'Shortcuts',
    rows: 2,
    cols: 4,
    tiles: [
      { id: 's1', cell: 0, label: 'Copy', icon: 'copy', action: 'Keys: Ctrl+C' },
      { id: 's2', cell: 1, label: 'Paste', icon: 'paste', action: 'Keys: Ctrl+V' },
      { id: 's3', cell: 2, label: 'Screenshot', icon: 'camera', action: 'Keys: Win+Shift+S' },
      { id: 's4', cell: 3, label: 'Lock', icon: 'lock', action: 'Keys: Win+L' },
      { id: 's5', cell: 4, label: 'Terminal', icon: 'terminal', action: 'Open app: Terminal' },
      { id: 's6', cell: 5, label: 'Browser', icon: 'globe', action: 'Open app: Browser' },
      { id: 's7', cell: 6, label: 'Calculator', icon: 'calc', action: 'Open app: Calculator' },
      { id: 's8', cell: 7, label: 'Documents', icon: 'folder', action: 'Open folder: Documents' },
    ],
  },
  {
    id: 'stats',
    title: 'Stats',
    rows: 2,
    cols: 3,
    tiles: [
      { id: 'p1', cell: 0, label: 'CPU', icon: 'cpu', action: 'Show reading: This PC · CPU', value: '23%' },
      { id: 'p2', cell: 1, label: 'Memory', icon: 'memory', action: 'Show reading: This PC · Memory used', value: '8.2 GB' },
      { id: 'p3', cell: 2, label: 'GPU', icon: 'gauge', action: 'Show reading: This PC · GPU', value: '41%' },
      { id: 'p4', cell: 3, label: 'Network send', icon: 'wifi', action: 'Show reading: This PC · Network send', value: '1.2 MB/s' },
      { id: 'p5', cell: 4, label: 'Disk', icon: 'disk', action: 'Show reading: This PC · Disk used', value: '62%' },
      { id: 'p6', cell: 5, label: 'CPU temp', icon: 'temp', action: 'Show reading: This PC · CPU temperature', value: '54°' },
    ],
  },
  {
    id: 'lights',
    title: 'Lights',
    rows: 2,
    cols: 4,
    tiles: [
      { id: 'l1', cell: 0, label: 'Desk lamp', icon: 'lamp', action: 'Home Assistant: Toggle light' },
      { id: 'l2', cell: 1, label: 'Ceiling', icon: 'bulb', action: 'Home Assistant: Toggle light' },
      { id: 'l3', cell: 4, label: 'Focus', icon: 'sun', action: 'Home Assistant: Run scene' },
      { id: 'l4', cell: 5, label: 'Movie', icon: 'moon', action: 'Home Assistant: Run scene' },
      { id: 'l5', cell: 7, label: 'All off', icon: 'power', action: 'Home Assistant: Toggle light' },
    ],
  },
];
