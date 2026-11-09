import { expect, type Page } from '@playwright/test';

export type Reading = 'opacity' | 'scale' | 'rotate' | 'translate-x' | 'matrix-a' | 'matrix-x' | 'matrix-y' | 'visible-width' | 'width';

export interface Probe {
  selector: string;
  reading: Reading;
  pseudo?: '::before' | '::after';
}

declare global {
  interface Window {
    motionTrace?: { series: number[][]; running: boolean };
    frameTrace?: { stamps: number[]; long: number[]; running: boolean };
  }
}

/**
 * Reads each probe on every animation frame until `stop()`. A frame where a probe's element is missing adds nothing
 * to that probe's series, so a popup's series ends with the last frame it was on screen.
 */
export async function trace(page: Page, probes: Probe[]) {
  await page.evaluate((probes) => {
    const number = (text: string) => Number.parseFloat(text);
    const matrix = (transform: string) =>
      transform === 'none' ? [1, 0, 0, 1, 0, 0] : transform.slice(transform.indexOf('(') + 1, -1).split(',').map(number);
    const words = (text: string) => {
      const parts: string[] = [];
      let depth = 0;
      let word = '';
      for (const ch of text) {
        if (ch === '(') depth++;
        if (ch === ')') depth--;
        if (ch === ' ' && depth === 0) {
          if (word) parts.push(word);
          word = '';
        } else word += ch;
      }
      if (word) parts.push(word);
      return parts;
    };
    const length = (text: string, box: number) =>
      text
        .replace(/calc\(|\)/g, '')
        .replace(/ - /g, ' + -')
        .split(' + ')
        .reduce((sum, term) => sum + (term.trim().endsWith('%') ? (number(term) * box) / 100 : number(term)), 0);
    const visibleWidth = (el: Element, clip: string) => {
      const box = el.getBoundingClientRect().width;
      const sides = words(clip.slice(clip.indexOf('(') + 1, clip.lastIndexOf(')')).split(' round ')[0].trim());
      return box - length(sides.length === 1 ? sides[0] : sides[1], box);
    };
    const read = (el: Element, probe: (typeof probes)[number]) => {
      const style = getComputedStyle(el, probe.pseudo ?? null);
      switch (probe.reading) {
        case 'opacity':
          return number(style.opacity);
        case 'scale':
          return style.scale === 'none' ? 1 : number(style.scale);
        case 'rotate':
          return style.rotate === 'none' ? 0 : number(style.rotate);
        case 'translate-x':
          return style.translate === 'none' ? 0 : number(style.translate);
        case 'matrix-a':
          return matrix(style.transform)[0];
        case 'matrix-x':
          return matrix(style.transform)[4];
        case 'matrix-y':
          return matrix(style.transform)[5];
        case 'visible-width':
          return visibleWidth(el, style.clipPath);
        case 'width':
          return el.getBoundingClientRect().width;
        default: {
          const unknown: never = probe.reading;
          return unknown;
        }
      }
    };
    const state = { series: probes.map(() => [] as number[]), running: true };
    window.motionTrace = state;
    const step = () => {
      if (!state.running) return;
      probes.forEach((probe, i) => {
        const el = document.querySelector(probe.selector);
        if (el) state.series[i].push(read(el, probe));
      });
      requestAnimationFrame(step);
    };
    step();
  }, probes);
  return {
    stop: () =>
      page.evaluate(() => {
        const state = window.motionTrace;
        if (!state) throw new Error('No trace is running.');
        state.running = false;
        return state.series;
      }),
  };
}

/** Plays every CSS transition on the page at `rate` times its speed, so a test can act halfway through one. */
export async function slowMotion(page: Page, rate = 0.1) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Animation.enable');
  await cdp.send('Animation.setPlaybackRate', { playbackRate: rate });
}

const progressOf = (values: number[], rest: number, goal: number) => values.map((v) => (v - rest) / (goal - rest));
const between = (p: number, low: number, high: number) => p > low + 0.01 && p < high - 0.01;

/** The series left `rest`, went part way toward `goal`, and came back to `rest` through the values in between. */
export function expectTurnedBack(values: number[], { rest, goal, removed = false }: { rest: number; goal: number; removed?: boolean }) {
  const progress = progressOf(values, rest, goal);
  const peak = Math.max(...progress);
  const turn = progress.indexOf(peak);
  expect(peak, `got part way: ${values.join(' ')}`).toBeGreaterThan(0.05);
  expect(peak, `turned back before arriving: ${values.join(' ')}`).toBeLessThan(0.99);
  const back = progress.slice(turn + 1);
  expect(back.filter((p) => between(p, 0, peak)).length, `came back without a jump: ${values.join(' ')}`).toBeGreaterThanOrEqual(2);
  expect(Math.abs(progress[progress.length - 1]), `ended at rest: ${values.join(' ')}`).toBeLessThan(removed ? 0.15 : 0.001);
}

/** The series went from `rest` to `goal` through the values in between. */
export function expectArrived(values: number[], { rest, goal }: { rest: number; goal: number }) {
  const progress = progressOf(values, rest, goal);
  expect(progress.filter((p) => between(p, 0, 1)).length, `moved through values in between: ${values.join(' ')}`).toBeGreaterThanOrEqual(2);
  expect(Math.abs(progress[progress.length - 1] - 1), `arrived: ${values.join(' ')}`).toBeLessThan(0.001);
}

/** The series only ever held one of `ends`: it jumped between them with nothing in between. */
export function expectJumped(values: number[], ends: [number, number]) {
  expect(values.length).toBeGreaterThan(0);
  expect(values.filter((v) => ends.every((end) => Math.abs(v - end) > 0.001)), `jumped: ${values.join(' ')}`).toEqual([]);
}

/** The series never left `value`. */
export function expectHeld(values: number[], value: number) {
  expectJumped(values, [value, value]);
}

/** Records the time of every animation frame, and every long animation frame (50ms or more), until `stop()`. */
export async function frames(page: Page) {
  await page.evaluate(() => {
    const state = { stamps: [] as number[], long: [] as number[], running: true };
    window.frameTrace = state;
    const observer = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) state.long.push(Math.round(entry.duration));
    });
    if (PerformanceObserver.supportedEntryTypes.includes('long-animation-frame')) observer.observe({ type: 'long-animation-frame' });
    const step = (now: number) => {
      if (!state.running) {
        observer.disconnect();
        return;
      }
      state.stamps.push(now);
      requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  });
  return {
    stop: () =>
      page.evaluate(() => {
        const state = window.frameTrace;
        if (!state) throw new Error('No frame trace is running.');
        state.running = false;
        return { stamps: state.stamps, long: state.long };
      }),
  };
}

/**
 * Frames a display refreshing every `budget` ms would miss between `stamps`: a gap of two refreshes is one missed
 * frame. Headless browsers run animation frames faster than any display, so the budget is fixed, not measured.
 */
export function droppedFrames(stamps: number[], budget = 1000 / 60) {
  let dropped = 0;
  for (let i = 1; i < stamps.length; i++) dropped += Math.max(0, Math.round((stamps[i] - stamps[i - 1]) / budget) - 1);
  return dropped;
}

export function longestGap(stamps: number[]) {
  return Math.round(Math.max(0, ...stamps.slice(1).map((t, i) => t - stamps[i])) * 10) / 10;
}
