import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { openCopyTile, picker } from './icon-picker-helpers.ts';

const folder = process.env.TD_BENCH ?? '';
// A demo URL to measure, such as `/?packs=mdi,logos` for installed Icon packs.
const path = process.env.TD_BENCH_PATH ?? '/';
const RUNS = 20;

type Run = { ms: number; inDom: number; press: number; cells: number };

async function timeOpen(page: Page): Promise<Run> {
  const done = page.evaluate(
    () =>
      new Promise<Run>((resolve, reject) => {
        const trigger = document.querySelector('[data-testid="icon-change"]');
        if (!trigger) return reject(new Error('no Change button'));
        let start = 0;
        let press = 0;
        trigger.addEventListener('pointerdown', (event) => (start = event.timeStamp), { capture: true, once: true });
        trigger.addEventListener('click', (event) => (press = event.timeStamp - start), { capture: true, once: true });
        const watch = new MutationObserver(() => {
          const cells = document.querySelectorAll('[data-testid="icon-picker"] [data-testid="icon-option"]').length;
          if (!start || cells === 0) return;
          watch.disconnect();
          const inDom = performance.now() - start;
          requestAnimationFrame(() => {
            const channel = new MessageChannel();
            channel.port1.onmessage = () => resolve({ ms: performance.now() - start, inDom, press, cells });
            channel.port2.postMessage(null);
          });
        });
        watch.observe(document.body, { childList: true, subtree: true });
      }),
  );
  await page.getByTestId('icon-change').click();
  return done;
}

const pct = (sorted: number[], p: number) => sorted[Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1)];
const round = (ms: number) => Math.round(ms * 10) / 10;

function summary(runs: Run[]) {
  const sorted = runs.map((r) => r.ms).sort((a, b) => a - b);
  const median = (pick: (run: Run) => number) => round(pct(runs.map(pick).sort((a, b) => a - b), 50));
  return {
    runs: runs.length,
    median: round(pct(sorted, 50)),
    p95: round(pct(sorted, 95)),
    min: round(sorted[0]),
    max: round(sorted[sorted.length - 1]),
    medianInDom: median((r) => r.inDom),
    medianPress: median((r) => r.press),
    cells: [...new Set(runs.map((r) => r.cells))],
    all: runs.map((r) => round(r.ms)),
  };
}

test.describe('icon picker open time', () => {
  test.skip(!folder, 'set TD_BENCH to a folder to measure');
  test.describe.configure({ mode: 'serial', timeout: 300_000 });

  test('first open after a page load, and repeat opens', async ({ browser }) => {
    const errors: string[] = [];
    const watched = async () => {
      const page = await browser.newPage();
      page.on('pageerror', (error) => errors.push(error.message));
      return page;
    };
    const cold: Run[] = [];
    for (let i = 0; i < RUNS; i++) {
      const page = await watched();
      await openCopyTile(page, path);
      await page.waitForTimeout(300);
      cold.push(await timeOpen(page));
      await page.close();
    }
    const page = await watched();
    await openCopyTile(page, path);
    const warm: Run[] = [];
    for (let i = 0; i < RUNS; i++) {
      await page.waitForTimeout(150);
      warm.push(await timeOpen(page));
      await expect(picker(page).getByTestId('icon-option').first()).toBeVisible();
      await page.keyboard.press('Escape');
      await expect(picker(page)).toHaveCount(0);
    }
    const result = { url: page.url(), errors, cold: summary(cold), warm: summary(warm) };
    writeFileSync(join(folder, 'open-time.json'), JSON.stringify(result, null, 2));
    expect(errors).toEqual([]);
    expect(result.cold.p95).toBeLessThan(200);
    expect(result.warm.p95).toBeLessThan(200);
  });
});
