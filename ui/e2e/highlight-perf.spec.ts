import { mkdtempSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page } from './test';
import { fixtures, git, openUrl } from './fixtures';

/**
 * The syntax highlighting probe: not part of the suite (it measures rather than asserts).
 * GITVOLT_PERF=1 runs it; it prints one JSON line per measurement. An 8,000-line TypeScript file
 * whose only change is near the bottom, so its diff opens scrolled there. Measures, from the click
 * that opens the diff: the time until its lines show, until every visible line with a `const` shows
 * it coloured, and the UI thread's busy time (CDP `TaskDuration`) until then and over the 3 s after
 * the click. Then the same on a reopen of that diff, and on another file once the grammar is warm.
 * The same text as a .txt (no highlighting) is the floor: the diff, layout and paint alone.
 */
test.skip(!process.env.GITVOLT_PERF, 'the highlighting perf probe runs with GITVOLT_PERF=1');
test.setTimeout(120_000);

/** ~8,000 lines of varied TypeScript: classes, interfaces, comments, template literals, regexes. */
function bigTs(lines: number): string[] {
  const out: string[] = [];
  for (let i = 0; out.length < lines; i++) {
    out.push(
      `/** Block ${i}: a doc comment with {@link Widget${i}} and \`code\`. */`,
      `export interface Shape${i} { id: number; label: string; tags?: readonly string[] }`,
      '',
      `export class Widget${i} implements Shape${i} {`,
      `  static readonly KIND = 'widget-${i}' as const;`,
      `  private cache = new Map<string, number>();`,
      `  constructor(public id: number, public label: string) {}`,
      '',
      `  render(prefix = '>'): string {`,
      `    const text = \`\${prefix} \${this.label} #\${this.id}\`;`,
      `    const re = /^(?:[a-z]+)-(\\d{2,})$/i;`,
      `    if (re.test(this.label)) return text.toUpperCase(); // keep it loud`,
      `    const n = this.cache.get(text) ?? ${i} * 2 + 0x1f;`,
      `    return \`\${text} (\${n.toFixed(2)})\`;`,
      `  }`,
      '',
      `  async load(url: string): Promise<Shape${i}[]> {`,
      `    const res = await fetch(url, { method: 'GET', headers: { accept: 'application/json' } });`,
      `    if (!res.ok) throw new Error(\`failed: \${res.status}\`);`,
      `    const body = (await res.json()) as { items: Shape${i}[] };`,
      `    return body.items.filter((s) => s.id > ${i} && s.label !== '');`,
      `  }`,
      `}`,
      '',
      `export function make${i}(count: number): Widget${i}[] {`,
      `  const all: Widget${i}[] = [];`,
      `  for (let k = 0; k < count; k++) {`,
      `    /* a block comment, ${i}:${'k'} */`,
      `    all.push(new Widget${i}(k, \`item-\${k}\`));`,
      `  }`,
      `  return all;`,
      `}`,
      '',
    );
  }
  return out.slice(0, lines);
}

async function busyMs(page: Page): Promise<() => Promise<number>> {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Performance.enable');
  const read = async () => (await cdp.send('Performance.getMetrics')).metrics.find((m) => m.name === 'TaskDuration')!.value;
  const start = await read();
  return async () => Math.round((await read() - start) * 1000);
}

/** Watches the modified editor from the next pointerdown: the ms from that click until the big
 * file's lines show (`shown`), and until every visible line containing `const ` has the keyword in
 * a coloured token, not `mtk1` (`coloured`). */
async function armColourWatch(page: Page): Promise<void> {
  await page.evaluate(() => {
    const w = window as unknown as { colouredAfter?: Promise<{ shown: number; coloured: number }> };
    w.colouredAfter = new Promise((resolve) => {
      document.addEventListener('pointerdown', () => {
        const t0 = performance.now();
        let shown = -1;
        const check = () => {
          const lines = [...document.querySelectorAll('.diff-panel .editor.modified .view-lines .view-line')].filter((l) => /const\s/.test(l.textContent ?? ''));
          if (shown < 0 && lines.length > 5) shown = Math.round(performance.now() - t0);
          const coloured = lines.length > 5 && lines.every((l) => [...l.querySelectorAll('span span')].some((s) => /^\s*const\s*$/.test(s.textContent ?? '') && !s.classList.contains('mtk1')));
          if (coloured) resolve({ shown, coloured: Math.round(performance.now() - t0) });
          else requestAnimationFrame(check);
        };
        requestAnimationFrame(check);
      }, { capture: true, once: true });
    });
  });
}

test('syntax highlighting perf probe', async ({ page }) => {
  const repo = join(mkdtempSync(join(fixtures.notRepo, 'hl-')), 'big');
  git(fixtures.notRepo, 'init', '-q', '-b', 'main', repo);
  const lines = bigTs(8000);
  writeFileSync(join(repo, 'big.ts'), lines.join('\n') + '\n');
  writeFileSync(join(repo, 'big.txt'), lines.join('\n') + '\n');
  const other = lines.map((l) => l.replaceAll('Widget', 'Gadget'));
  writeFileSync(join(repo, 'other.ts'), other.join('\n') + '\n');
  writeFileSync(join(repo, 'small.ts'), 'export const a = 1;\n');
  git(repo, 'add', 'big.ts', 'big.txt', 'other.ts', 'small.ts');
  git(repo, 'commit', '-q', '-m', 'Add the big file');
  lines[7900] = `    const text = 'changed near the bottom';`;
  writeFileSync(join(repo, 'big.ts'), lines.join('\n') + '\n');
  writeFileSync(join(repo, 'big.txt'), lines.join('\n') + '\n');
  other[7900] = lines[7900];
  writeFileSync(join(repo, 'other.ts'), other.join('\n') + '\n');
  writeFileSync(join(repo, 'small.ts'), 'export const a = 2;\n');
  git(repo, 'commit', '-q', '-am', 'Edit far down');

  await page.goto(openUrl(repo));
  await page.getByRole('row').filter({ hasText: 'Edit far down' }).click();
  const row = (path: string) => page.getByRole('option').and(page.locator(`[data-path="${path}"]`));
  // Warm Monaco and the highlighter on the small file first: the probe measures the big file.
  await row('small.ts').click();
  await expect(page.locator('.diff-panel .editor.modified .view-line').first()).toBeVisible({ timeout: 15_000 });
  await page.waitForTimeout(1000);

  const busy3s = async (path: string) => {
    const busy = await busyMs(page);
    await row(path).click();
    await page.waitForTimeout(3000);
    return busy();
  };
  console.log(JSON.stringify({ probe: 'plain big.txt (floor)', uiBusy3sMs: await busy3s('big.txt') }));
  await row('small.ts').click();
  await page.waitForTimeout(1500);

  // The grammar's first big file (its rules compile as they're first used), the same diff again,
  // then another file once the grammar is warm.
  for (const [pass, path] of [['open', 'big.ts'], ['reopen', 'big.ts'], ['open another, warm grammar', 'other.ts']]) {
    if (pass !== 'open') {
      await row('small.ts').click();
      await page.waitForTimeout(1500);
    }
    await armColourWatch(page);
    const busy = await busyMs(page);
    await row(path).click();
    const t = await page.evaluate(() => (window as unknown as { colouredAfter: Promise<{ shown: number; coloured: number }> }).colouredAfter);
    const busyToColour = await busy();
    await page.waitForTimeout(Math.max(0, 3000 - t.coloured));
    console.log(JSON.stringify({ probe: pass, linesShownMs: t.shown, visibleColouredMs: t.coloured, uiBusyToColourMs: busyToColour, uiBusy3sMs: await busy() }));
  }
});
