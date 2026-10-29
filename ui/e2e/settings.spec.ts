import type { Page } from '@playwright/test';
import { DENSITY_STORAGE_KEY } from '../src/theme/density';
import { freshFixture, git, openUrl } from './fixtures';
import { expect, test, confirmArmed, armedOverlay } from './test';

const dialog = (page: Page) => page.getByRole('dialog', { name: 'Settings' });
const open = async (page: Page, path = freshFixture('basic')) => {
  await page.goto(openUrl(path));
  await expect(page.getByRole('grid', { name: 'Commit graph' })).toBeVisible();
  return path;
};
/** Opens the dropdown button and picks a row from the app's menu. */
const pick = async (page: Page, button: string, row: string | RegExp) => {
  await dialog(page).getByRole('button', { name: button }).click();
  await page.getByRole('menuitem', { name: row }).click();
};
const openSettings = async (page: Page) => {
  await page.keyboard.press('Control+,');
  await expect(dialog(page)).toBeVisible();
};
/** A page's nav item (while searching, its match count follows the name). */
const nav = (page: Page, name: string) => dialog(page).getByRole('navigation', { name: 'Settings pages' }).getByRole('button', { name: new RegExp(`^${name}\\s*\\d*$`) });
/** Settings closed first (the step before left it open). */
const closeSettings = async (page: Page) => {
  await page.keyboard.press('Escape');
  await expect(dialog(page)).toHaveCount(0);
};

test.describe('settings', () => {
  // Each `test.step` below was a test of its own, paying for a page load; they run in an order
  // where each starts from what it needs (Settings closed, or open on the page it reads).
  test('the Settings overlay: opening and closing, its controls and pages, what each setting changes, search, reset', async ({ page }) => {
    await test.step('Ctrl+, opens it and Esc closes it', async () => {
      await open(page);
      await openSettings(page);
      await expect(dialog(page).getByLabel('Search settings')).toBeFocused();
      await closeSettings(page);
    });
    await test.step('K102: the gear opens settings; K103: a label click leaves the dropdown closed and its background matches the inputs', async () => {
      await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
      await expect(dialog(page)).toBeVisible();
      await nav(page, 'Graph').click();
      await dialog(page).getByText('Date format', { exact: true }).click();
      await expect(page.getByRole('menu')).toHaveCount(0);
      await dialog(page).getByText('Date format', { exact: true }).hover();
      const bg = (sel: string) => dialog(page).locator(sel).first().evaluate((el) => getComputedStyle(el).backgroundColor);
      const select = await bg('button.select');
      expect(select).toBe(await bg('.setting-control input:not([type="checkbox"]):not([type="radio"])'));
    });
    await test.step('K94: the close button is the shared icon button, not a bordered box', async () => {
      const close = dialog(page).getByRole('button', { name: 'Close settings' });
      await expect(close).toHaveClass(/icon-button/);
      await expect(close).toHaveCSS('border-top-width', '0px');
      await expect(close).toHaveCSS('border-top-left-radius', '3px');
      await close.click();
      await expect(dialog(page)).toHaveCount(0);
    });
    await test.step('K93: the pages are separate, the nav groups name the profile and repository, and the palette deep link opens the right page and focuses the setting', async () => {
      await openSettings(page);
      await nav(page, 'General').click();
      const pages = dialog(page).getByRole('navigation', { name: 'Settings pages' });
      await expect(pages.getByText('GitVolt', { exact: true })).toBeVisible();
      await expect(pages.getByText(/^Profile · /)).toBeVisible();
      await expect(pages.getByText(/^Repository · /)).toBeVisible();
      await expect(dialog(page).getByRole('region', { name: 'General' })).toBeVisible();
      await expect(dialog(page).getByRole('region', { name: 'Fetch and push' })).toHaveCount(0);
      await nav(page, 'Fetch and push').click();
      await expect(dialog(page).getByRole('region', { name: 'Fetch and push' })).toBeVisible();
      await expect(dialog(page).getByRole('region', { name: 'General' })).toHaveCount(0);
      await closeSettings(page);
      await openSettings(page);
      await expect(dialog(page).getByRole('region', { name: 'Fetch and push' })).toBeVisible();
      await closeSettings(page);
      await page.keyboard.press('Control+p');
      await page.getByLabel('Command palette query').fill('#sticky');
      await page.keyboard.press('Enter');
      await expect(dialog(page).getByRole('region', { name: 'Diffs and editors' })).toBeVisible();
      await expect(dialog(page).getByLabel('Sticky scroll', { exact: true })).toBeFocused();
    });
    await test.step('K92: a dropdown opens on click and with the keyboard, changes its value, and Esc closes only the menu', async () => {
      await closeSettings(page);
      await openSettings(page);
      await nav(page, 'Graph').click();
      const date = dialog(page).getByRole('button', { name: 'Date format' });
      await date.click();
      await expect(page.getByRole('menu')).toBeVisible();
      await page.getByRole('menuitem', { name: '26/09/2026 15:14' }).click();
      await expect(date).toHaveText('26/09/2026 15:14');
      await expect(page.getByRole('menu')).toHaveCount(0);
      // The keyboard half is a keyboard user's: the pointer isn't parked where the next menu opens (a
      // stationary pointer under a menu that appears fires a late hover that moves the active row).
      await page.mouse.move(0, 0);
      await date.focus();
      await page.keyboard.press('ArrowDown');
      await expect(page.getByRole('menu')).toBeVisible();
      await page.keyboard.press('Escape');
      await expect(page.getByRole('menu')).toHaveCount(0);
      await expect(dialog(page)).toBeVisible();
      await expect(date).toBeFocused();
      await page.keyboard.press('Enter');
      await expect(page.getByRole('menu')).toBeVisible();
      await page.keyboard.press('ArrowDown');
      await page.keyboard.press('Enter');
      await expect(date).toHaveText('09/26/2026 3:14 PM');
      await expect(dialog(page)).toBeVisible();
    });
    await test.step('the date format applies to the graph', async () => {
      await closeSettings(page);
      const dateCell = page.getByRole('row').filter({ hasText: 'Fix typo' }).locator('[data-col="date"]');
      await expect(dateCell).toContainText(/ (AM|PM)$/);
      await openSettings(page);
      await nav(page, 'Graph').click();
      await pick(page, 'Date format', '2026-09-26 15:14');
      await expect(dateCell).toHaveText(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/);
      await pick(page, 'Date format', '26/09/2026 15:14');
      await expect(dateCell).toHaveText(/^\d{2}\/\d{2}\/\d{4} \d{2}:\d{2}$/);
      await dialog(page).getByRole('switch', { name: 'Show relative dates' }).click();
      await closeSettings(page);
      await expect(dateCell).toHaveText(/ago|yesterday|just now/);
      await dateCell.hover();
      await expect(page.getByRole('tooltip')).toContainText(/^\d{2}\/\d{2}\/\d{4} \d{2}:\d{2}/);
      await openSettings(page);
      await nav(page, 'Graph').click();
      await dialog(page).getByRole('switch', { name: 'Show relative dates' }).click();
      await closeSettings(page);
      await expect(dateCell).toHaveText(/^\d{2}\/\d{2}\/\d{4} \d{2}:\d{2}$/);
      await openSettings(page);
      await nav(page, 'Graph').click();
    });
    await test.step('Reset all (Advanced) arms in place first; Esc disarms it and leaves Settings open', async () => {
      await closeSettings(page);
      await openSettings(page);
      await nav(page, 'Graph').click();
      await pick(page, 'Date format', '09/26/2026 3:14 PM');
      await nav(page, 'Advanced').click();
      await dialog(page).getByRole('button', { name: 'Reset all settings…' }).click();
      const armed = armedOverlay(page, 'Click again to reset every GitVolt setting');
      await expect(armed).toBeVisible();
      await page.keyboard.press('Escape');
      await expect(armed).toBeHidden();
      await expect(dialog(page)).toBeVisible();
      await dialog(page).getByRole('button', { name: 'Reset all settings…' }).click();
      await confirmArmed(armed);
      await nav(page, 'Graph').click();
      await expect(dialog(page).getByLabel('Date format')).toHaveText('2026-09-26 @ 3:14 PM');
    });
    await test.step('density persists behind its store; sticky scroll is an app setting', async () => {
      await closeSettings(page);
      await openSettings(page);
      await nav(page, 'Appearance').click();
      await dialog(page).getByRole('radio', { name: 'Compact' }).check();
      await expect.poll(() => page.evaluate((k) => localStorage.getItem(k), DENSITY_STORAGE_KEY)).toBe('compact');
      await nav(page, 'Diffs and editors').click();
      const sticky = dialog(page).getByLabel('Sticky scroll', { exact: true });
      await sticky.check();
      await expect(sticky).toBeChecked();
      // Not in localStorage any more: the old key was moved into the app settings and removed.
      expect(await page.evaluate(() => localStorage.getItem('gitvolt.editorSettings.v1'))).toBeNull();
      // The explanation is always shown, under the label.
      await expect(dialog(page).getByText(/Pins the enclosing scope's first line/)).toBeVisible();
    });
    await test.step('search: "avatar" lists matches in place, a switch changes there, Esc clears', async () => {
      await closeSettings(page);
      await openSettings(page);
      await nav(page, 'Fetch and push').click();
      await page.keyboard.press('Control+f');
      await expect(dialog(page).getByLabel('Search settings')).toBeFocused();
      await page.keyboard.type('avatar');
      await expect(dialog(page).getByRole('heading', { name: '3 settings match "avatar"' })).toBeVisible();
      await expect(nav(page, 'Graph').locator('.settings-nav-count')).toHaveText('2');
      await expect(nav(page, 'General')).toHaveAttribute('data-dim', 'true');
      const gravatar = dialog(page).getByLabel('From Gravatar');
      await expect(gravatar).toBeChecked();
      await gravatar.click();
      await expect(gravatar).not.toBeChecked();
      await page.keyboard.press('Escape');
      await expect(dialog(page).getByLabel('Search settings')).toHaveValue('');
      await expect(dialog(page).getByRole('region', { name: 'Fetch and push' })).toBeVisible();
    });
    await test.step('a changed setting shows its ↺; ↺ restores it', async () => {
      const row = dialog(page).locator('[data-setting-id="prune"]');
      const reset = row.getByRole('button', { name: 'Restore the default' });
      await expect(reset).toBeHidden();
      await dialog(page).getByLabel('Prune deleted remote branches on fetch').click();
      await expect(reset).toBeVisible();
      await reset.click();
      await expect(dialog(page).getByLabel('Prune deleted remote branches on fetch')).toBeChecked();
      await expect(reset).toBeHidden();
    });
    await test.step('a Custom editor command the guard refuses shows its error inline; a valid one is kept', async () => {
      await closeSettings(page);
      await openSettings(page);
      await nav(page, 'Editor and folders').click();
      await pick(page, 'Default editor', 'Custom command…');
      const command = dialog(page).getByLabel('Custom editor command');
      await command.fill('sh -c "geany {file}"');
      await expect(dialog(page).getByRole('alert')).toContainText("can't contain {file}, {line} or {repo}");
      await command.fill('/bin/echo --goto {file}:{line}');
      await expect(dialog(page).getByRole('alert')).toHaveCount(0);
      // Kept by the core's settings, as the host override below shows across a reload. (The field
      // left, so the next step's Esc is the dialog's.)
      await command.press('Tab');
      await expect(command).not.toBeFocused();
    });
    await test.step('branch presets: a new preset, a rule and its live count; Copy as JSON then Paste preset adds "(2)"', async () => {
      await closeSettings(page);
      await openSettings(page);
      await nav(page, 'Branch presets').click();
      await dialog(page).getByRole('button', { name: '+ New preset' }).click();
      const name = dialog(page).getByRole('textbox', { name: 'Name' });
      await expect(name).toBeFocused();
      await name.fill('Payments');
      await name.press('Enter');
      const all = dialog(page).getByRole('list', { name: 'All repositories' });
      await expect(all.getByRole('button', { name: 'Payments', exact: true })).toHaveAttribute('aria-pressed', 'true');
      await dialog(page).getByRole('button', { name: '+ Add rule' }).click();
      const patterns = dialog(page).getByRole('textbox', { name: 'Rule 1 patterns' });
      await patterns.fill('feature/*');
      await patterns.press('Enter');
      await expect(dialog(page).getByRole('listitem', { name: 'Rule 1' })).toContainText(/\d+ branch(es)?✕$/);
      await expect(dialog(page).getByText(/^In .+ now: \d+ shown · \d+ collapsed · \d+ hidden · \d+ merged branches folded$/)).toBeVisible();
      await dialog(page).getByRole('button', { name: 'Copy as JSON' }).click();
      await dialog(page).getByRole('button', { name: 'Paste preset' }).click();
      await expect(all.getByRole('button', { name: 'Payments (2)', exact: true })).toHaveAttribute('aria-pressed', 'true');
      // Alt+↑ on its grip moves it up, past the dialog's claim on every key.
      await all.getByRole('button', { name: 'Reorder Payments (2)' }).focus();
      await page.keyboard.press('Alt+ArrowUp');
      await expect(all.locator('.bp-name')).toHaveText(['Payments (2)', 'Payments']);
    });
    await test.step('the commit limit reloads the graph window', async () => {
      await closeSettings(page);
      await openSettings(page);
      await nav(page, 'Graph').click();
      const rows = page.getByRole('grid', { name: 'Commit graph' }).getByRole('row');
      const before = await rows.count();
      expect(before).toBeGreaterThan(3);
      const limit = dialog(page).getByLabel('Commits loaded in the graph', { exact: true });
      await limit.fill('3');
      await limit.press('Enter');
      await expect.poll(() => rows.count()).toBeLessThan(before);
    });
  });

  test('file colours: the file list menu opens the page with the pattern, and the new scope tints the file', async ({ page }) => {
    await open(page, freshFixture('details'));
    await test.step("file colours: colour a file's type from its menu", async () => {
      await page.getByRole('row').filter({ hasText: 'Rename guide and update assets' }).click();
      const row = page.locator('[role="treeitem"][data-path="logo.png"], [role="option"][data-path="logo.png"]');
      await row.click({ button: 'right', position: { x: 40, y: 10 } });
      await page.getByRole('menuitem', { name: 'Colour *.png files…' }).click();
      await expect(dialog(page)).toBeVisible();
      await expect(dialog(page).getByRole('heading', { name: 'File colours' })).toBeVisible();
      await expect(dialog(page).getByRole('textbox', { name: 'Pattern', exact: true })).toHaveValue('*.png');
      await dialog(page).getByRole('button', { name: 'New scope', exact: true }).click();
      await closeSettings(page);
      await expect(row).toHaveAttribute('data-tinted', 'true');
    });
  });

  test('a host override switches a generic remote to GitLab, in the graph and the sidebar', async ({ page }) => {
    const repo = freshFixture('basic');
    git(repo, 'remote', 'set-url', 'origin', 'git@code.example.com:acme/shop.git');
    await open(page, repo);
    const icons = page.locator('[aria-label="remote origin"]');
    await expect(icons.first()).toHaveAttribute('data-host-kind', 'generic');
    await openSettings(page);
    await nav(page, 'Hosts').click();
    await pick(page, 'Forge type for code.example.com', 'GitLab');
    await page.keyboard.press('Escape');
    await expect(icons.first()).toHaveAttribute('data-host-kind', 'gitlab');
    // Every origin icon follows, the sidebar's folder included.
    for (const icon of await icons.all()) await expect(icon).toHaveAttribute('data-host-kind', 'gitlab');
    // And it is the profile's: it survives a reload.
    await page.reload();
    await expect(page.getByRole('grid', { name: 'Commit graph' })).toBeVisible();
    await expect(icons.first()).toHaveAttribute('data-host-kind', 'gitlab');
  });
});
