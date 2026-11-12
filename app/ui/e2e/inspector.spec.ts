import { expect, test, type Page } from '@playwright/test';
import { addTile, emitKeys, openStarter, press, script, send, setCommand, storedAction, watchInPageFetch } from './inspector-helpers.ts';
import { status, waitEvent } from './layout-helpers.ts';

const form = (page: Page) => page.getByTestId('action-form');
const kind = (page: Page, type: string) => page.getByTestId('action-type').selectOption(type);

async function saved(page: Page, id: string, action: unknown) {
  await expect(form(page)).not.toHaveAttribute('data-invalid');
  await send(page);
  expect(await storedAction(page, 'shortcuts', id)).toEqual(action);
}

test.beforeEach(async ({ page }) => {
  await openStarter(page);
});

test('sets up and sends every action kind, keeping the program across an app and a command', async ({ page }) => {
  const id = await addTile(page, 'key');

  await page.getByTestId('action-key').selectOption({ label: 'C' });
  await page.getByTestId('action-mod-ctrl').click();
  await page.getByTestId('action-mod-shift').click();
  await expect(page.getByTestId('action-shortcut')).toHaveText('Presses Ctrl + Shift + C on this PC.');
  await saved(page, id, { type: 'key', usage: 6, mods: ['ctrl', 'shift'] });

  await kind(page, 'consumer');
  await page.getByTestId('action-media').selectOption({ label: 'Mute' });
  await saved(page, id, { type: 'consumer', usage: 226 });

  await kind(page, 'mod');
  await page.getByTestId('action-modifier').selectOption('shift');
  await saved(page, id, { type: 'mod', mod: 'shift' });

  await kind(page, 'launch');
  await page.getByTestId('action-path').fill('notepad.exe');
  await saved(page, id, { type: 'launch', path: 'notepad.exe', args: [], cwd: '' });

  await kind(page, 'command');
  await expect(page.getByTestId('action-program')).toHaveValue('notepad.exe');
  await setCommand(page, 'robocopy', ['C:\\Projects', 'D:\\Backup', '/MIR']);
  await page.getByTestId('action-cwd').fill('C:\\Projects');
  await page.getByTestId('action-shell').click();
  await expect(page.getByTestId('command-line')).toHaveText('robocopy C:\\Projects D:\\Backup /MIR');
  await saved(page, id, { type: 'command', program: 'robocopy', args: ['C:\\Projects', 'D:\\Backup', '/MIR'], cwd: 'C:\\Projects', shell: true });

  await kind(page, 'open');
  await page.getByTestId('action-target').fill('https://example.com');
  await saved(page, id, { type: 'open', target: 'https://example.com' });

  await kind(page, 'page');
  await page.getByTestId('action-page').selectOption('media');
  await saved(page, id, { type: 'page', page: 'media' });

  await kind(page, 'none');
  await saved(page, id, { type: 'none' });
});

test('explains an invalid value next to its field, and Send to Device refuses until it is fixed', async ({ page }) => {
  const id = await addTile(page, 'launch');
  await expect(form(page)).toHaveAttribute('data-invalid', 'true');
  await expect(page.getByTestId('field-error')).toHaveText('Choose an app.');
  await expect(page.getByTestId('action-path')).toHaveAttribute('aria-invalid', 'true');

  await page.getByTestId('tile-label').fill(' ');
  await expect(page.getByTestId('field-error')).toHaveText(['Give the Tile a label.', 'Choose an app.']);
  await page.getByTestId('tile-label').fill('Notes');

  await page.getByTestId('page-entry').filter({ hasText: 'Media' }).click();
  await page.getByTestId('push').click();
  await waitEvent(page, 'refused');
  await expect(status(page).locator('.editor-status-text')).toHaveText('Choose an app.');
  await expect(page.getByTestId('page-entry').filter({ hasText: 'Shortcuts' })).toHaveAttribute('aria-current', 'true');
  await expect(page.getByTestId('tile-fields')).toHaveAttribute('data-id', id);
  expect(await storedAction(page, 'shortcuts', id)).toBeUndefined();

  await page.getByTestId('action-path').fill('notepad.exe');
  await expect(page.getByTestId('field-error')).toHaveCount(0);
  await saved(page, id, { type: 'launch', path: 'notepad.exe', args: [], cwd: '' });
});

test('a pressed command Tile waits for approval on this PC, and the wait clears when the dialog answers', async ({ page }) => {
  const id = await addTile(page, 'command');
  await page.getByTestId('tile-label').fill('Backup');
  await setCommand(page, 'robocopy', ['C:\\Projects', 'D:\\Backup']);
  await expect(page.getByTestId('approval-note')).toContainText('Open TouchDeck shows the exact command and asks before it runs.');
  await send(page);

  await press(page, 'shortcuts', id, 'command');
  const pending = page.getByTestId('approval-pending');
  await expect(pending).toContainText('Waiting for approval on this PC');
  await expect(pending).toContainText('Backup on the Shortcuts Page of Desk');
  await expect(pending.getByTestId('approval-program')).toHaveText('robocopy');
  await expect(pending.getByTestId('approval-args').locator('li')).toHaveText(['C:\\Projects', 'D:\\Backup']);
  await expect(pending.getByRole('button')).toHaveCount(1);

  await watchInPageFetch(page, /\/approvals\/[^/]+\/show$/);
  await pending.getByTestId('approval-show').click();
  await expect(page.locator('body')).toHaveAttribute('data-in-page-fetch', '204');

  await script(page, ['deny']);
  await expect(pending).toHaveCount(0);
  await page.getByTestId('action-log-panel').locator('summary').click();
  await expect(page.getByTestId('action-entry').first()).toHaveAttribute('data-status', 'denied');
});

test('the pending card writes an invisible character in an argument as an escape, and the Tile keeps the raw value', async ({ page }) => {
  const id = await addTile(page, 'command');
  await setCommand(page, 'robocopy', ['report‮gpj.exe']);
  await send(page);
  expect(await storedAction(page, 'shortcuts', id)).toMatchObject({ args: ['report‮gpj.exe'] });

  await press(page, 'shortcuts', id, 'command');
  const pending = page.getByTestId('approval-pending');
  await expect(pending.getByTestId('approval-args').locator('li')).toHaveText(['report\\u{202E}gpj.exe']);
  expect(await pending.textContent()).not.toContain('‮');
  await script(page, ['deny']);
  await expect(pending).toHaveCount(0);
});

test('lists an always-allowed command and removes it', async ({ page }) => {
  const id = await addTile(page, 'command');
  await setCommand(page, 'robocopy', ['C:\\Projects', 'D:\\Backup']);
  await send(page);
  await script(page, ['always']);
  await press(page, 'shortcuts', id, 'command');

  const allowed = page.getByTestId('allowed-action');
  await expect(allowed).toHaveCount(1);
  await expect(allowed).toHaveAttribute('data-mine', 'true');
  await page.getByTestId('action-log-panel').locator('summary').click();
  await expect(page.getByTestId('action-entry').first()).toHaveAttribute('data-status', 'exited');

  await page.getByTestId('allowed-remove').click();
  await expect(page.getByTestId('allowed-actions')).toHaveCount(0);
  await script(page, ['never']);
  await press(page, 'shortcuts', id, 'command');
  await expect(page.getByTestId('approval-pending')).toBeVisible();
});

test("Key test shows the Device's key reports, suppressed while the dialog waits, and the keys this PC gets", async ({ page }) => {
  const id = await addTile(page, 'command');
  await setCommand(page, 'robocopy', ['C:\\Projects']);
  await send(page);

  await page.locator('[data-testid="tile"][data-id="copy"]').click();
  await page.getByTestId('key-test-panel').locator('summary').click();
  await emitKeys(page, [{ type: 'kbd', mods: 1, keys: [6] }, { type: 'kbd', mods: 0, keys: [] }]);
  await expect(page.getByTestId('key-report')).toHaveCount(2);
  await expect(page.getByTestId('key-report').first()).toHaveAttribute('data-outcome', 'typed');

  await press(page, 'shortcuts', id, 'command');
  await expect(page.getByTestId('approval-pending')).toBeVisible();
  await emitKeys(page, [{ type: 'con', usage: 233 }]);
  await expect(page.getByTestId('key-report')).toHaveCount(3);
  await expect(page.getByTestId('key-report').last()).toHaveAttribute('data-outcome', 'suppressed');

  await page.getByTestId('key-monitor').click();
  await page.keyboard.press('Control+c');
  await expect(page.getByTestId('key-monitor')).toHaveValue('Control c');
  await expect(page.getByTestId('key-event')).toHaveCount(4);

  await page.getByTestId('key-monitor-clear').click();
  await expect(page.getByTestId('key-report')).toHaveCount(0);
  await expect(page.getByTestId('key-event')).toHaveCount(0);
});

test('focus is visible on the inspector fields', async ({ page }) => {
  await addTile(page, 'command');
  const reach = async (testId: string) => {
    const el = page.getByTestId(testId);
    await el.focus();
    await page.keyboard.press('Tab');
    await page.keyboard.press('Shift+Tab');
    await expect(el).toBeFocused();
    return el.evaluate((node) => ({ visible: node.matches(':focus-visible'), ring: getComputedStyle(node).outlineStyle !== 'none' }));
  };
  for (const id of ['tile-label', 'tile-size-1x1', 'action-type', 'action-program', 'action-args', 'action-cwd', 'action-shell']) {
    expect({ id, ...(await reach(id)) }).toEqual({ id, visible: true, ring: true });
  }
});
