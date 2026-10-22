import type { Page } from '@playwright/test';
import { THEME_IDS, THEMES } from '../src/theme/themes';
import { armedOverlay, confirmArmed } from './armed';
import { fixtures, openUrl } from './fixtures';
import { expect, test } from './test';

// Themes (spec §12.1, plan 1D Task 2): applying one, switching from the palette and Settings, lane
// overrides; the editor follows the app theme; the screenshot matrix of every theme (Chromium).
test.use({ timezoneId: 'UTC' });

const grid = (page: Page) => page.getByRole('grid', { name: 'Commit graph' });
const theme = (page: Page) => page.locator('html').getAttribute('data-theme');
const rgb = (hex: string) => `rgb(${[1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)).join(', ')})`;
const canvasHash = (page: Page) => page.getByTestId('graph-canvas').first().evaluate((c: HTMLCanvasElement) => {
  const d = c.getContext('2d')!.getImageData(0, 0, c.width, c.height).data;
  let h = 0;
  for (let i = 0; i < d.length; i += 97) h = (h * 31 + d[i]) | 0;
  return h;
});
const dialog = (page: Page) => page.getByRole('dialog', { name: 'Settings' });

// Each `test.step` below was a test of its own, paying for a page load; they run in an order
// where each starts from what it needs (a step that needs a plain URL loads it).
test('themes: Default Dark by default; a lane edit makes a custom theme; ?theme=; the palette, saved and mirrored; the hamburger opens the picker', async ({ page }) => {
  await test.step('Default Dark by default', async () => {
    await page.goto(openUrl(fixtures.basic));
    await expect(grid(page)).toBeVisible();
    expect(await theme(page)).toBe('default-dark');
    await expect(page.locator('body')).toHaveCSS('background-color', 'rgb(28, 30, 35)');
  });
  await test.step('Appearance: editing a colour makes "Nord (Custom)"; it survives a reload; Delete returns to Nord', async () => {
    const appBg = () => page.locator('html').evaluate((el) => el.style.getPropertyValue('--app-bg0'));
    await page.keyboard.press('Control+,');
    await dialog(page).getByRole('button', { name: 'Appearance' }).click();
    await dialog(page).getByRole('radio', { name: 'Nord', exact: true }).click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'nord');
    await expect(dialog(page).getByText('Change any colour below to make your own copy.')).toBeVisible();
    const field = dialog(page).getByRole('textbox', { name: 'Window background', exact: true });
    await field.fill('#202020');
    await field.press('Enter');
    await expect(dialog(page).getByRole('radio', { name: 'Nord (Custom)' })).toHaveAttribute('aria-checked', 'true');
    await expect(dialog(page).getByLabel('Theme name')).toHaveValue('Nord (Custom)');
    await expect.poll(appBg).toBe('#202020');
    await page.reload();
    await expect(grid(page)).toBeVisible();
    await expect.poll(appBg).toBe('#202020');
    await page.keyboard.press('Control+,');
    await dialog(page).getByRole('button', { name: 'Appearance' }).click();
    await expect(dialog(page).getByRole('radio', { name: 'Nord (Custom)' })).toHaveAttribute('aria-checked', 'true');
    await dialog(page).getByRole('button', { name: 'Delete', exact: true }).click();
    await confirmArmed(armedOverlay(page, 'Click again to delete Nord (Custom)'));
    await expect(dialog(page).getByRole('radio', { name: 'Nord', exact: true })).toHaveAttribute('aria-checked', 'true');
    await expect(dialog(page).getByRole('radio', { name: 'Nord (Custom)' })).toHaveCount(0);
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'nord');
    await expect.poll(appBg).toBe(THEMES.nord.colors['app-bg0']);
    await page.keyboard.press('Escape');
    await expect(dialog(page)).toBeHidden();
  });
  await test.step('a lane edit on Nord makes "Nord (Custom)", recolours that lane\'s chips and canvas, and survives a reload', async () => {
    const chip = page.locator('.ref-labels').first();
    const laneColor = () => chip.evaluate((el) => (el as HTMLElement).style.getPropertyValue('--lane-color'));
    await page.keyboard.press('Control+,');
    await dialog(page).getByRole('button', { name: 'Appearance' }).click();
    await dialog(page).getByRole('radio', { name: 'Nord', exact: true }).click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'nord');
    await expect.poll(async () => THEMES.nord.graph.includes(await laneColor())).toBe(true);
    const lane = THEMES.nord.graph.indexOf(await laneColor());
    const before = await canvasHash(page);
    await dialog(page).getByRole('button', { name: `Lane ${lane + 1} colour` }).click();
    const hex = page.getByRole('dialog', { name: `Lane ${lane + 1} colour` }).getByLabel('Hex');
    await hex.fill('#123456');
    await hex.press('Enter');
    await expect(dialog(page).getByRole('radio', { name: 'Nord (Custom)' })).toHaveAttribute('aria-checked', 'true');
    await expect(page.locator('html')).toHaveAttribute('data-theme', /^custom-[0-9a-f]{8}$/);
    await expect.poll(laneColor).toBe('#123456');
    await expect.poll(() => canvasHash(page)).not.toBe(before);
    // Saved, and mirrored with the custom theme itself for the first paint.
    await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('gitvolt.theme.v1') ?? 'null')?.custom?.name)).toBe('Nord (Custom)');
    await page.reload();
    await expect(grid(page)).toBeVisible();
    await expect(page.locator('html')).toHaveAttribute('data-theme', /^custom-[0-9a-f]{8}$/);
    await expect.poll(laneColor).toBe('#123456');
  });
  await test.step('?theme= applies a theme to the page and the canvas', async () => {
    const dark = await canvasHash(page);
    await page.goto(`${openUrl(fixtures.basic)}&theme=light`);
    await expect(grid(page)).toBeVisible();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
    await expect(page.locator('body')).toHaveCSS('background-color', rgb(THEMES.light.colors['app-bg0']));
    await expect.poll(() => canvasHash(page)).not.toBe(dark);
  });
  await test.step('switching theme from the palette repaints the canvas, and the choice survives a reload', async () => {
    await page.goto(openUrl(fixtures.basic));
    await expect(grid(page)).toBeVisible();
    const before = await canvasHash(page);
    await page.keyboard.press('Control+P');
    await page.keyboard.type('>Theme: Solarized Light');
    await page.keyboard.press('Enter');
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'solarized-light');
    await expect(page.locator('body')).toHaveCSS('background-color', rgb(THEMES['solarized-light'].colors['app-bg0']));
    await expect.poll(() => canvasHash(page)).not.toBe(before);
    // Saved (the backend's settings), and mirrored for the first paint (R3).
    await expect.poll(() => page.evaluate(() => localStorage.getItem('gitvolt.theme.v1'))).toBe(JSON.stringify({ id: 'solarized-light', kind: 'light', bg: THEMES['solarized-light'].colors['app-bg0'] }));
    await page.reload();
    await expect(grid(page)).toBeVisible();
    expect(await theme(page)).toBe('solarized-light');
  });
  await test.step('the hamburger shows one "Theme…" row; it opens Settings on the theme cards, which switch the theme', async () => {
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('menuitem', { name: 'View' }).click();
    await expect(page.getByRole('menuitem', { name: /^Theme: / })).toHaveCount(0);
    await page.getByRole('menuitem', { name: 'Theme…' }).click();
    await expect(dialog(page)).toBeVisible();
    await expect(dialog(page).getByRole('region', { name: 'Appearance' })).toBeVisible();
    await expect(dialog(page).getByRole('radio', { name: 'Solarized Light' })).toBeFocused();
    await dialog(page).getByRole('radio', { name: 'Nord', exact: true }).click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'nord');
    await expect(dialog(page)).toHaveCSS('background-color', rgb(THEMES.nord.colors['panel-bg1']));
  });
});

test("index.html paints the mirrored theme's background and scheme on its own, before the app's module runs (I1)", async ({ page }) => {
  await page.addInitScript((bg) => localStorage.setItem('gitvolt.theme.v1', JSON.stringify({ id: 'light', kind: 'light', bg })), THEMES.light.colors['app-bg0']);
  // No app module at all: only the inline <head> script can have set these.
  // (The dev server's /src/main.tsx, or the production build's entry chunk.)
  await page.route(/\/(src\/main\.tsx|assets\/index-[^/]*\.js)(\?|$)/, (r) => r.abort());
  await page.goto(openUrl(fixtures.basic));
  const root = await page.evaluate(() => ({ theme: document.documentElement.dataset.theme, scheme: document.documentElement.style.colorScheme, bg: document.documentElement.style.getPropertyValue('--app-bg0') }));
  expect(root).toEqual({ theme: 'light', scheme: 'light', bg: THEMES.light.colors['app-bg0'] });
});


const COMMIT = 'Rename guide and update assets';
const diffRegion = (page: Page) => page.getByRole('region', { name: 'Diff' });
const editorBg = (page: Page) => diffRegion(page).locator('.monaco-editor .monaco-editor-background').first().evaluate((e) => getComputedStyle(e).backgroundColor);

async function selectCommit(page: Page) {
  await page.getByRole('row').filter({ hasText: COMMIT }).click();
  await expect(page.getByTestId('file-counts')).toBeVisible();
}
async function openDiff(page: Page) {
  await selectCommit(page);
  await page.locator('[role="option"][data-path="src/app.php"], [role="treeitem"][data-path="src/app.php"]').click();
  await expect(diffRegion(page).locator('.editor.modified .line-insert').first()).toBeVisible({ timeout: 15_000 });
}
// One page, one editor: the font size step uses the diff the theme step opened.
test('the open editor: a theme switch repaints it without re-creating it (R6); Settings > Appearance resizes it live, clamped to 8-32', async ({ page }) => {
  await test.step("switching theme repaints the open editor in the new theme's background, without re-creating it (R6)", async () => {
    await page.goto(openUrl(fixtures.details));
    await openDiff(page);
    const host = diffRegion(page).locator('.monaco-host').first();
    await host.evaluate((el) => { (el as HTMLElement).dataset.keep = '1'; });
    await expect.poll(() => editorBg(page)).toBe(rgb(THEMES['default-dark'].colors['app-bg0'])); // today's #1c1e23
    const keyword = diffRegion(page).locator('.editor.modified .view-line span span').filter({ hasText: /^function$/ }).first();
    await expect(keyword).toHaveCSS('color', 'rgb(86, 156, 214)'); // dark-plus
    await page.keyboard.press('Control+P');
    await page.keyboard.type('>Theme: Darcula');
    await page.keyboard.press('Enter');
    await expect.poll(() => editorBg(page)).toBe(rgb(THEMES.darcula.colors['app-bg0']));
    await page.keyboard.press('Control+P');
    await page.keyboard.type('>Theme: Solarized Light');
    await page.keyboard.press('Enter');
    await expect.poll(() => editorBg(page)).toBe(rgb(THEMES['solarized-light'].colors['app-bg0']));
    // The tokens follow: the highlight worker's, in the new theme's colours.
    await expect(keyword).toHaveCSS('color', 'rgb(88, 110, 117)'); // solarized-light
    await expect(host).toHaveAttribute('data-keep', '1');
  });
  await test.step('the editor font size row (Settings > Appearance) resizes the open diff live, clamped to 8-32', async () => {
    const size = () => diffRegion(page).locator('.view-line').first().evaluate((e) => getComputedStyle(e).fontSize);
    await expect.poll(size).toBe('13px');
    await page.keyboard.press('Control+,');
    await dialog(page).getByRole('button', { name: 'Appearance', exact: true }).click();
    const box = dialog(page).getByLabel('Editor font size');
    await box.fill('18');
    await box.press('Enter');
    await expect.poll(size).toBe('18px');
    await box.fill('99');
    await box.press('Enter');
    await expect(box).toHaveValue('32');
    await expect.poll(size).toBe('32px');
  });
});

test.describe('theme screenshots', () => {
  // Pixel baselines: opt-in (`just e2e-shots`), recorded on one machine; fonts and antialiasing
  // elsewhere would fail the gate (W2-A review).
  test.skip(!process.env.GITVOLT_E2E_SHOTS, 'pixel baselines run with GITVOLT_E2E_SHOTS=1 (just e2e-shots)');
  test.skip(({ browserName }) => browserName !== 'chromium', 'screenshot baselines are Chromium-only');
  for (const id of THEME_IDS) {
    test(`${id}: graph, details, split diff`, async ({ page }) => {
      await page.setViewportSize({ width: 1600, height: 900 });
      await page.goto(`${openUrl(fixtures.details)}&theme=${id}`);
      await expect(grid(page)).toBeVisible();
      const shot = { mask: [page.locator('img')], maxDiffPixelRatio: 0.01 }; // avatars may load differently between runs
      await selectCommit(page);
      await expect(page.locator('.graph-panel')).toHaveScreenshot(`graph-${id}.png`, shot);
      await expect(page.locator('.right-panel')).toHaveScreenshot(`details-${id}.png`, shot);
      await page.getByRole('option').and(page.locator('[data-path="src/app.php"]')).click();
      await expect(diffRegion(page).locator('.editor.modified .line-insert').first()).toBeVisible({ timeout: 15_000 });
      await diffRegion(page).getByRole('button', { name: 'Split' }).click();
      await expect.poll(() => editorBg(page)).toBe(rgb(THEMES[id].colors['app-bg0']));
      await expect(diffRegion(page)).toHaveScreenshot(`diff-split-${id}.png`, shot);
    });
  }
});
