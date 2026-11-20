import { mkdtempSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page } from './test';
import { fixtures, git, openUrl } from './fixtures';

/**
 * The large-diff probe: not part of the suite (it measures rather than asserts). GITVOLT_PERF=1
 * runs it; it prints one JSON line per measurement. Generated fictional files: a 30,000-line lock
 * file with 6,000 changed lines, an 8,000-line reformat (every indented line), a typical
 * 10,000-line file with 200 changed lines, a 20,000-line file whose functions swapped places in
 * pairs, and a 50,000-line one (3.7 M characters both sides) with a few changes. Measures, from the
 * click that opens a file, the ms until its lines are on screen: on the first open, on a reopen,
 * and for a small file opened 1.5 s after leaving the lock file (its diff may still be computing).
 * Then the page's JS heap (and the editor worker's) after the 50,000-line diff is closed with Esc.
 */
test.skip(!process.env.GITVOLT_PERF, 'the large-diff perf probe runs with GITVOLT_PERF=1');
test.setTimeout(900_000);

let seed = 7;
const rand = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
const hex = (n: number) => Array.from({ length: n }, () => '0123456789abcdef'[Math.floor(rand() * 16)]).join('');

/** A package-lock-like file: `blocks` five-line entries, `changed` of them with a new version,
 * URL and hash (three changed lines each). */
function lockFile(blocks: number, changed: number): [string, string] {
  const a: string[] = [];
  const b: string[] = [];
  const every = Math.floor(blocks / changed);
  for (let i = 0; i < blocks; i++) {
    const name = `pkg-${hex(6)}-${i}`;
    const v = `${i % 9}.${i % 13}.${i % 7}`;
    const block = [`    "node_modules/${name}": {`, `      "version": "${v}",`, `      "resolved": "https://registry.example.test/${name}/-/${name}-${v}.tgz",`, `      "integrity": "sha512-${hex(40)}"`, '    },'];
    a.push(...block);
    if (i % every === 0 && i / every < changed) {
      const v2 = `${i % 9}.${(i % 13) + 1}.0`;
      b.push(block[0]!, `      "version": "${v2}",`, `      "resolved": "https://registry.example.test/${name}/-/${name}-${v2}.tgz",`, `      "integrity": "sha512-${hex(40)}"`, block[4]!);
    } else b.push(...block);
  }
  return [a.join('\n') + '\n', b.join('\n') + '\n'];
}

/** `lines` lines of TypeScript, its functions named after `tag` (what the probe looks for on screen). */
function code(tag: string, lines: number): string[] {
  const out: string[] = [];
  for (let i = 0; out.length < lines; i++) {
    out.push(`export function ${tag}${i}(input: Record<string, number>, limit = ${i % 50}): number {`, '  let total = 0;', '  for (const [key, value] of Object.entries(input)) {', `    if (key.startsWith('k${i % 17}') && value > limit) total += value * ${i % 7}; // ${tag}`, '  }', `  return total; // ${tag} ${hex(8)}`, '}', '');
  }
  return out.slice(0, lines);
}
const text = (lines: string[]) => lines.join('\n') + '\n';

function makeRepo(): string {
  const repo = join(mkdtempSync(join(fixtures.notRepo, 'dperf-')), 'big');
  git(fixtures.notRepo, 'init', '-q', '-b', 'main', repo);
  const [lockA, lockB] = lockFile(6000, 2000);
  const reformat = code('reformatHandler', 8000);
  const typical = code('typicalHandler', 10_000);
  const reorder = code('reorderHandler', 20_000);
  const big = Array.from({ length: 50_000 }, (_, i) => `bigLine ${String(i).padStart(6, '0')} lorem ipsum dolor sit`);
  const files: Record<string, [string, string]> = {
    'package-lock.json': [lockA, lockB],
    'reformat.ts': [text(reformat), text(reformat.map((l) => l.replace(/^( +)/, '$1$1')))],
    'typical.ts': [text(typical), text(typical.map((l, i) => (i % 50 === 3 ? `    if (key === 'typicalHandler changed ${i}') total -= ${i};` : l)))],
    'reorder.ts': [text(reorder), text(Array.from({ length: reorder.length / 16 }, (_, k) => [...reorder.slice(k * 16 + 8, k * 16 + 16), ...reorder.slice(k * 16, k * 16 + 8)]).flat())],
    'big.txt': [text(big), text(big.map((l, i) => (i % 5000 === 7 ? `${l} changed` : l)))],
    'small.ts': ['export const smallValue = 1;\n', 'export const smallValue = 2;\n'],
  };
  for (const [path, [a]] of Object.entries(files)) writeFileSync(join(repo, path), a);
  git(repo, 'add', ...Object.keys(files));
  git(repo, 'commit', '-q', '-m', 'Add the big files');
  for (const [path, [, b]] of Object.entries(files)) writeFileSync(join(repo, path), b);
  git(repo, 'commit', '-q', '-am', 'Change the big files');
  return repo;
}

/** What each file shows on every line: how the probe tells it's on screen. */
const MARK: Record<string, string> = {
  'package-lock.json': 'node_modules/pkg', 'reformat.ts': 'reformatHandler', 'typical.ts': 'typicalHandler',
  'reorder.ts': 'reorderHandler', 'big.txt': 'bigLine', 'small.ts': 'smallValue',
};

/** From the next pointerdown: the ms until the diff panel's editor shows `mark` (and isn't hidden). */
async function armShown(page: Page, mark: string): Promise<void> {
  await page.evaluate((m) => {
    const w = window as unknown as { shownAfter?: Promise<number> };
    w.shownAfter = new Promise((resolve) => {
      document.addEventListener('pointerdown', () => {
        const t0 = performance.now();
        const check = () => {
          const panel = [...document.querySelectorAll('.diff-panel')].find((p) => p.getClientRects().length > 0);
          const host = panel?.querySelector<HTMLElement>('.monaco-host');
          const lines = [...(panel?.querySelectorAll('.editor.modified .view-lines .view-line') ?? [])];
          if (host && host.style.opacity !== '0' && lines.some((l) => l.textContent?.includes(m))) resolve(Math.round(performance.now() - t0));
          else if (performance.now() - t0 > 30_000) resolve(-1);
          else requestAnimationFrame(check);
        };
        requestAnimationFrame(check);
      }, { capture: true, once: true });
    });
  }, mark);
}

// Made once for both probes, and only when they run.
const repo = process.env.GITVOLT_PERF ? makeRepo() : '';
const log = (o: object) => console.log(JSON.stringify(o));

/** The repo's commit selected; `open` clicks a file and resolves to the ms until it's on screen. */
async function start(page: Page) {
  await page.goto(openUrl(repo));
  await page.getByRole('row').filter({ hasText: 'Change the big files' }).click();
  const row = (path: string) => page.getByRole('option').and(page.locator(`[data-path="${path}"]`));
  const open = async (path: string) => {
    await armShown(page, MARK[path]!);
    await row(path).click();
    return page.evaluate(() => (window as unknown as { shownAfter: Promise<number> }).shownAfter);
  };
  return { row, open };
}

test('large-diff perf probe', async ({ page }) => {
  const { row, open } = await start(page);
  // Warm Monaco and the highlighter on the small file first.
  await open('small.ts');
  await page.waitForTimeout(1000);

  for (const path of ['package-lock.json', 'reformat.ts', 'typical.ts', 'reorder.ts']) {
    const first = await open(path);
    // The whole-file change a timed-out diff falls back to: one insert decoration per line shown.
    await page.waitForTimeout(500);
    const inserted = await page.locator('.diff-panel .editor.modified .line-insert').count();
    await open('small.ts');
    await page.waitForTimeout(1500);
    const reopen = await open(path);
    log({ probe: path, firstMs: first, reopenMs: reopen, insertDecorationsShown: inserted });
    await open('small.ts');
    await page.waitForTimeout(1500);
  }

  // A small file right after the lock file: its diff waits for the lock file's unless that's cancelled.
  await open('typical.ts');
  await page.waitForTimeout(1500);
  await armShown(page, MARK['package-lock.json']!);
  await row('package-lock.json').click();
  await page.waitForTimeout(1500);
  const next = await open('small.ts');
  log({ probe: 'small.ts 1.5 s after opening package-lock.json', ms: next });
});

test('closed-diff heap probe', async ({ page, browser }) => {
  const { open } = await start(page);
  await open('small.ts');
  await page.waitForTimeout(1000);
  // The heap once a 50,000-line diff is closed.
  const cdp = await page.context().newCDPSession(page);
  const heap = async () => {
    await cdp.send('HeapProfiler.collectGarbage');
    return Math.round((await cdp.send('Runtime.getHeapUsage')).usedSize / 1e5) / 10;
  };
  const workerHeap = async () => {
    const bcdp = await browser.newBrowserCDPSession();
    const { targetInfos } = await bcdp.send('Target.getTargets');
    const out: number[] = [];
    for (const t of targetInfos.filter((x) => x.type === 'worker')) {
      const { sessionId } = await bcdp.send('Target.attachToTarget', { targetId: t.targetId, flatten: false });
      const reply = new Promise<number>((resolve) => {
        bcdp.on('Target.receivedMessageFromTarget', (e) => {
          const m = JSON.parse(e.message) as { id?: number; result?: { usedSize: number } };
          if (e.sessionId === sessionId && m.id === 2 && m.result) resolve(m.result.usedSize);
        });
      });
      await bcdp.send('Target.sendMessageToTarget', { sessionId, message: JSON.stringify({ id: 1, method: 'HeapProfiler.collectGarbage' }) });
      await bcdp.send('Target.sendMessageToTarget', { sessionId, message: JSON.stringify({ id: 2, method: 'Runtime.getHeapUsage' }) });
      out.push(Math.round(await Promise.race([reply, new Promise<number>((r) => setTimeout(() => r(-1e5), 3000))]) / 1e5) / 10);
      await bcdp.send('Target.detachFromTarget', { sessionId });
    }
    await bcdp.detach();
    return out;
  };
  await page.keyboard.press('Escape');
  await expect(page.locator('.diff-panel')).toBeHidden();
  await page.waitForTimeout(2000);
  const baseline = { pageMB: await heap(), workersMB: await workerHeap() };
  const bigMs = await open('big.txt');
  await page.waitForTimeout(2000);
  const shownBig = { pageMB: await heap(), workersMB: await workerHeap() };
  await page.keyboard.press('Escape');
  await expect(page.locator('.diff-panel')).toBeHidden();
  await page.waitForTimeout(2000);
  const closed2s = { pageMB: await heap(), workersMB: await workerHeap() };
  await page.waitForTimeout(13_000);
  const closed15s = { pageMB: await heap(), workersMB: await workerHeap() };
  log({ probe: 'heap (MB): before, big.txt shown, closed +2 s, closed +15 s', bigMs, baseline, shownBig, closed2s, closed15s });
});
