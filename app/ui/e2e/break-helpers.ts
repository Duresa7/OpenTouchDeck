import type { Page } from '@playwright/test';

/** One element whose content leaves its box, or text cut off with no way to read the rest. */
export interface Spill {
  path: string;
  text: string;
  kind: 'overflow' | 'clipped' | 'truncated-without-title' | 'outside-viewport';
  axis: 'x' | 'y';
  by: number;
}

// A Device screen, on the canvas, in a Page thumbnail or in a Preset preview, clips its Tiles' text as the Firmware does.
const UNCHECKED = ['svg', 'input', 'textarea', 'select', '[data-demo-toggle]', '.device-scene', '.scene', '[data-testid="deck-screen"]', '.page-thumb', '.preset-screen', '.sonner-toast', '[hidden]'].join(', ');

/**
 * Every element whose content is wider than its box when the box lets it spill, or with a child or a run of text
 * cut off when the box clips. A scroll container is left out on the axis it scrolls. Text cut with an ellipsis
 * passes only when a `title` on it or an ancestor shows the whole value. `allowed` lists selectors whose cut is
 * accepted.
 */
export function spills(page: Page, allowed: readonly string[] = []): Promise<Spill[]> {
  return page.evaluate(
    ({ skip, allowed }) => {
      const found: Spill[] = [];
      const pathOf = (el: Element) => {
        const parts: string[] = [];
        for (let at: Element | null = el; at && parts.length < 4 && at !== document.body; at = at.parentElement) {
          const test = at.getAttribute('data-testid');
          const cls = [...at.classList].slice(0, 2).join('.');
          parts.unshift(`${at.tagName.toLowerCase()}${test ? `[data-testid=${test}]` : ''}${cls ? `.${cls}` : ''}`);
        }
        return parts.join(' > ');
      };
      const visible = (el: Element) => {
        const box = el.getBoundingClientRect();
        if (box.width <= 1 || box.height <= 1) return false;
        const style = getComputedStyle(el);
        return style.visibility !== 'hidden' && style.display !== 'none' && Number(style.opacity) > 0;
      };
      const scrolls = (value: string) => value === 'auto' || value === 'scroll';
      const clips = (value: string) => value === 'hidden' || value === 'clip';
      const inScroller = (el: Element) => {
        for (let at = el.parentElement; at && at !== document.body; at = at.parentElement) {
          if (scrolls(getComputedStyle(at).overflowX)) return true;
        }
        return false;
      };
      const hasText = (el: Element) => [...el.childNodes].some((node) => node.nodeType === Node.TEXT_NODE && /\S/u.test(node.textContent ?? ''));
      const boxesIn = (el: Element): DOMRect[] => {
        const boxes: DOMRect[] = [];
        for (const node of el.childNodes) {
          if (node.nodeType === Node.TEXT_NODE && /\S/u.test(node.textContent ?? '')) {
            const range = document.createRange();
            range.selectNodeContents(node);
            boxes.push(range.getBoundingClientRect());
          } else if (node instanceof Element && !node.matches(skip) && visible(node) && getComputedStyle(node).position !== 'fixed') {
            boxes.push(node.getBoundingClientRect());
          }
        }
        return boxes;
      };
      const pastEdge = (el: Element, axis: 'x' | 'y') => {
        const outer = el.getBoundingClientRect();
        const start = axis === 'x' ? outer.left + el.clientLeft : outer.top + el.clientTop;
        const end = start + (axis === 'x' ? el.clientWidth : el.clientHeight);
        let by = 0;
        for (const box of boxesIn(el)) {
          by = Math.max(by, axis === 'x' ? Math.max(start - box.left, box.right - end) : Math.max(start - box.top, box.bottom - end));
        }
        return by > 1 ? Math.round(by) : 0;
      };
      const width = document.documentElement.clientWidth;
      for (const el of document.body.querySelectorAll('*')) {
        if (el.closest(skip) || !visible(el) || allowed.some((selector) => el.matches(selector))) continue;
        const style = getComputedStyle(el);
        const add = (kind: Spill['kind'], axis: Spill['axis'], by: number) => found.push({ path: pathOf(el), text: (el.textContent ?? '').trim().slice(0, 60), kind, axis, by });
        const box = el.getBoundingClientRect();
        if ((box.left < -1 || box.right > width + 1) && !el.closest('[inert], [aria-hidden="true"]') && !inScroller(el)) {
          add('outside-viewport', 'x', Math.round(Math.max(-box.left, box.right - width)));
        }
        if (style.display === 'inline' || style.display === 'contents') continue;
        if (clips(style.overflowX) && style.textOverflow === 'ellipsis' && hasText(el)) {
          if (el.scrollWidth > el.clientWidth + 1 && !el.closest('[title]')) add('truncated-without-title', 'x', el.scrollWidth - el.clientWidth);
          continue;
        }
        if (!clips(style.overflowX) && !scrolls(style.overflowX) && el.scrollWidth > el.clientWidth + 1) add('overflow', 'x', el.scrollWidth - el.clientWidth);
        for (const axis of ['x', 'y'] as const) {
          if (!clips(axis === 'x' ? style.overflowX : style.overflowY)) continue;
          const by = pastEdge(el, axis);
          if (by) add('clipped', axis, by);
        }
      }
      return found;
    },
    { skip: UNCHECKED, allowed },
  );
}

/** The elements matching `selector` that the nav rail covers. A hidden one, such as a folded Pages column's, can't be. */
export function underRail(page: Page, selector: string): Promise<string[]> {
  return page.evaluate((selector) => {
    const rail = document.querySelector('.nav-rail')?.getBoundingClientRect();
    if (!rail) return [];
    const covered = (box: DOMRect) => box.width > 1 && box.height > 1 && box.left < rail.right && box.right > rail.left && box.top < rail.bottom && box.bottom > rail.top;
    const shown = (el: Element) => el.checkVisibility({ visibilityProperty: true });
    return [...document.querySelectorAll(selector)].filter((el) => shown(el) && covered(el.getBoundingClientRect())).map((el) => `${el.getAttribute('data-testid') ?? el.tagName.toLowerCase()} “${(el.textContent ?? '').trim().slice(0, 40)}”`);
  }, selector);
}

/** How far the page scrolls sideways; 0 when nothing pushes it wider than the window. */
export const sideScroll = (page: Page) => page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);

/** Collects console errors and uncaught exceptions from the moment it is called. */
export function consoleErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(`${message.text()} ${message.location().url}`);
  });
  page.on('pageerror', (error) => errors.push(error.message));
  return errors;
}
