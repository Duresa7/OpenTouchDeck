import { expect, type Page } from '@playwright/test';
import type { Action, Layout } from '../src/lib/layout/model.ts';
import { openLayout, waitEvent } from './layout-helpers.ts';

export const DESK = 'mock0001';

export async function openStarter(page: Page, path = '/') {
  await openLayout(page, path);
  await page.getByTestId('start-starter').click();
  await page.getByTestId('page-tab').filter({ hasText: 'Shortcuts' }).click();
}

/** Add Tile opens on Integrations while one offers Preset tiles; actions live in the Actions tab. */
export async function actionsTab(page: Page) {
  await page.getByTestId('add-tab-actions').click();
  await expect(page.getByTestId('add-tab-actions')).toHaveAttribute('aria-selected', 'true');
}

export async function addTile(page: Page, kind: string, row = 2, col = 2): Promise<string> {
  await page.locator(`[data-testid="empty-cell"][data-row="${row}"][data-col="${col}"]`).click();
  await actionsTab(page);
  await page.locator(`[data-testid="add-kind"][data-kind="${kind}"]`).click();
  const id = await page.getByTestId('tile-fields').getAttribute('data-id');
  if (id === null) throw new Error(`Add Tile placed no ${kind} Tile`);
  return id;
}

export async function send(page: Page) {
  await page.getByTestId('push').click();
  await waitEvent(page, 'pushed');
}

export async function storedAction(page: Page, pageId: string, tile: string, device = DESK): Promise<Action | undefined> {
  const record: { layouts: { landscape: Layout | null } } = await page.evaluate(async (id) => (await fetch(`/api/v1/test/devices/${id}/record`)).json(), device);
  const grid = record.layouts.landscape?.pages.find((p) => p.id === pageId);
  return grid?.kind === 'grid' ? grid.tiles.find((t) => t.id === tile)?.action : undefined;
}

async function post(page: Page, path: string, body: unknown) {
  const status = await page.evaluate(async ([to, text]) => (await fetch(to, { method: 'POST', body: text })).status, [path, JSON.stringify(body)] as const);
  expect(status).toBe(204);
}

export const press = (page: Page, pageId: string, tile: string, kind: string, device = DESK) =>
  post(page, `/api/v1/test/devices/${device}/press`, { page: pageId, tile, kind });

export const script = (page: Page, answers: string[]) => post(page, '/api/v1/test/approvals/script', { answers });

export const emitKeys = (page: Page, events: Record<string, unknown>[], device = DESK) =>
  post(page, `/api/v1/test/devices/${device}/emit`, { events: events.map((event) => ({ m: 'key.report', ...event })) });

export async function watchInPageFetch(page: Page, route: RegExp) {
  await page.evaluate((source) => {
    const through = window.fetch;
    window.fetch = async (input, init) => {
      const response = await through(input, init);
      if (new RegExp(source).test(String(input))) document.body.dataset.inPageFetch = `${document.body.dataset.inPageFetch ?? ''} ${response.status}`.trim();
      return response;
    };
  }, route.source);
}

export async function setCommand(page: Page, program: string, args: string[]) {
  await page.getByTestId('action-program').fill(program);
  await page.getByTestId('action-args').fill(args.join('\n'));
}
