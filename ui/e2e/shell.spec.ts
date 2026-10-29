import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { freshFixture, git, harnessHttp, harnessWs, openUrl } from './fixtures';
import { expect, test, type Page } from './test';

const openAll = (...paths: string[]) => `/?${paths.map((p) => `repo=${encodeURIComponent(p)}`).join('&')}`;
const graph = (page: Page) => page.getByRole('grid', { name: 'Commit graph' });
const row = (page: Page, text: string) => page.getByRole('row').filter({ hasText: text });

/** The backend's id for the repo at `path` (the one the app opened: repos are shared by path). */
function repoIdOf(page: Page, path: string): Promise<number> {
  return page.evaluate(({ url, path }) => new Promise<number>((resolve, reject) => {
    const ws = new WebSocket(url);
    ws.onopen = () => ws.send(JSON.stringify({ id: 1, req: { method: 'openRepo', params: { path } } }));
    ws.onmessage = (e) => {
      const m = JSON.parse(String(e.data)) as { id?: number; ok?: { id: number }; err?: unknown };
      if (m.id !== 1) return;
      ws.close();
      if (m.ok) resolve(m.ok.id);
      else reject(new Error(JSON.stringify(m.err)));
    };
  }), { url: harnessWs, path });
}

async function emit(page: Page, event: object) {
  const res = await page.request.post(`${harnessHttp}/test/emit`, { data: event });
  expect(res.ok()).toBe(true);
}

test.describe('shell', () => {
  // One repo and page for these (each was a test of its own, paying for a page load).
  test('a refs-updated event refreshes in place, a file change updates the WIP row live, the open tabs come back after a reload, and the title bar', async ({ page }) => {
    const repo = freshFixture('basic');
    await page.goto(openUrl(repo));
    await test.step('a refs-updated event refreshes in place, keeping the selected commit selected', async () => {
      const fix = row(page, 'Fix typo');
      await fix.click();
      await expect(fix).toHaveAttribute('aria-selected', 'true');
      git(repo, 'switch', '-q', '-c', 'e2e-branch', 'main');
      writeFileSync(join(repo, 'e2e.txt'), 'x\n');
      git(repo, 'add', 'e2e.txt');
      git(repo, 'commit', '-q', '-m', 'Commit from outside');
      await emit(page, { type: 'refsUpdated', repo: await repoIdOf(page, repo) });
      await expect(row(page, 'Commit from outside')).toBeVisible({ timeout: 5000 });
      await expect(row(page, 'Fix typo')).toHaveAttribute('aria-selected', 'true');
      // The details panel still shows it.
      await expect(page.getByRole('complementary', { name: 'Commit details' })).toContainText('Fix typo');
    });
    // The watcher's own event (no emitted one): the real file system to the WIP row.
    await test.step('a file change in the worktree updates the WIP row live', async () => {
      const mainWip = page.getByRole('row').filter({ hasText: '// WIP' }).filter({ hasNotText: 'wt-hotfix' });
      await expect(mainWip.getByTestId('wip-counts')).toHaveAccessibleName('1 modified');
      writeFileSync(join(repo, 'e2e-new-file.txt'), 'hello\n');
      await expect(mainWip.getByTestId('wip-counts')).toHaveAccessibleName('1 modified · 1 added', { timeout: 5000 });
    });
    await test.step('restores open tabs after a reload', async () => {
      await expect(graph(page)).toBeVisible();
      await page.evaluate(() => window.__gb!.flush());
      // The reloaded page is the app's window on Linux, for the title bar step below: what the
      // app's initialization script hands over (gitvolt-app chrome.rs). There's no window here,
      // so its calls are recorded (`window.__gbWindowCalls`).
      await page.addInitScript(() => {
        window.__GITVOLT_WINDOW__ = { frame: 'custom', platform: 'linux', layout: 'appmenu:minimize,maximize,close', doubleClick: 'toggle-maximize', middleClick: 'lower', rightClick: 'menu' };
      });
      await page.goto('/');
      await expect(graph(page)).toBeVisible();
      await expect(page.getByText("Merge branch 'feature/login'")).toBeVisible();
      await expect(page.locator('.tab-page')).toHaveCount(1);
    });
    await test.step("GitVolt's own title bar: ☰ moves up into it, its buttons and clicks ask the window", async () => {
      const bar = page.locator('.title-bar');
      const calls = () => page.evaluate(() => window.__gbWindowCalls?.splice(0) ?? []);
      await expect(bar.getByRole('button', { name: 'Menu' })).toBeVisible();
      await expect(page.locator('.tab-bar').getByRole('button', { name: 'Menu' })).toHaveCount(0);
      await expect(bar.locator('.title-bar-title')).toHaveText(/^GitVolt — /);
      // 30 px, right above the tab bar.
      const [title, tabs] = await Promise.all([bar.boundingBox(), page.locator('.tab-bar').boundingBox()]);
      expect([title!.y, title!.height, tabs!.y]).toEqual([0, 30, 30]);
      // ☰ opens the app menu from up here.
      await bar.getByRole('button', { name: 'Menu' }).click();
      await expect(page.getByRole('menu', { name: 'Menu' })).toBeVisible();
      await page.keyboard.press('Escape');
      await bar.getByRole('button', { name: 'Minimize' }).click();
      await bar.getByRole('button', { name: 'Maximize' }).click();
      await expect(page.locator('.window-edges')).toHaveCount(0);
      await bar.getByRole('button', { name: 'Restore' }).click();
      await expect(page.locator('.window-edge')).toHaveCount(8);
      await page.locator('.window-edge-se').dispatchEvent('mousedown', { button: 0 });
      expect(await calls()).toEqual(['minimize', 'toggleMaximize', 'toggleMaximize', 'startResizeDragging:SouthEast']);
      // Its title drags the window once the pointer moves; a double click maximizes; a middle
      // click lowers; a right click asks for the window manager's menu and, with none here,
      // shows GitVolt's.
      const box = (await bar.locator('.title-bar-title').boundingBox())!;
      await page.mouse.move(box.x + box.width / 2, box.y + 15);
      await page.mouse.down();
      await page.mouse.move(box.x + box.width / 2 + 40, box.y + 20);
      await page.mouse.up();
      await bar.locator('.title-bar-title').dblclick();
      await bar.locator('.title-bar-title').click({ button: 'middle' });
      await bar.locator('.title-bar-title').click({ button: 'right' });
      await expect(page.getByRole('menuitem', { name: 'Close' })).toBeVisible();
      await page.keyboard.press('Escape');
      expect(await calls()).toEqual(['startDragging', 'toggleMaximize', 'windowManager:lower', 'windowManager:menu']);
      await bar.getByRole('button', { name: 'Close' }).click();
      expect(await calls()).toEqual(['close']);
    });
  });

  // Two repos as two tabs, only the active one shown, Ctrl+Tab: tabs.spec.ts's two-tab test.
  test('files open in two tabs share the one diff editor across switches; Ctrl+W closes the file, then the tab', async ({ page }) => {
    const a = freshFixture('basic');
    const b = freshFixture('basic');
    await page.goto(openAll(a, b));
    await expect(page.locator('.tab-page')).toHaveCount(2);
    const diff = page.getByRole('region', { name: 'Diff' });
    const path = diff.getByTestId('diff-path');
    // The modified side's text lines (not a deleted-lines view zone).
    const lines = diff.locator('.editor.modified .lines-content > .view-lines');
    const openFirstFile = async (summary: string) => {
      await row(page, summary).click();
      await expect(page.getByRole('listbox', { name: 'Changed files' })).toBeVisible();
      await graph(page).press('Enter');
      // The first open also loads (and, on a fresh dev server, compiles) the editor's chunk.
      await expect(lines).toBeVisible({ timeout: 30_000 });
    };
    // Tab b (active): a file of "Fix typo".
    await openFirstFile('Fix typo');
    const inB = { path: await path.innerText(), text: await lines.innerText() };
    // Tab a: a file of "Add readme".
    await page.keyboard.press('Control+Tab');
    await expect(diff).toHaveCount(0);
    await openFirstFile('Add readme');
    const inA = { path: await path.innerText(), text: await lines.innerText() };
    expect(inA.text).not.toBe(inB.text);
    // Back to b: its own file, in the one editor (moved over).
    await page.keyboard.press('Control+Shift+Tab');
    await expect(path).toHaveText(inB.path);
    await expect(lines).toHaveText(inB.text);
    // And a again.
    await page.keyboard.press('Control+Tab');
    await expect(path).toHaveText(inA.path);
    await expect(lines).toHaveText(inA.text);
    // Ctrl+W: the open file first (back to the graph, selection kept), then the tab.
    await page.keyboard.press('Control+w');
    await expect(diff).toHaveCount(0);
    await expect(row(page, 'Add readme')).toHaveAttribute('aria-selected', 'true');
    await expect(page.locator('.tab-page')).toHaveCount(2);
    await page.keyboard.press('Control+w');
    await expect(page.locator('.tab-page')).toHaveCount(1);
    // b is left, its file still open.
    await expect(path).toHaveText(inB.path);
    await expect(lines).toHaveText(inB.text);
  });
});
