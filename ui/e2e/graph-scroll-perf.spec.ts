import { mkdtempSync } from 'node:fs';
import { join } from 'node:path';
import { budgetApplies, expect, test } from './test';
import { fixtures, openUrl } from './fixtures';
import { avatarPng, makePerfRepo } from './perfRepo';

/**
 * The graph's scroll performance probe: not part of the suite (it takes ~30 s and measures
 * rather than asserts much), and a branch preset switch's budget on the same repo, which is. GITVOLT_PERF=1 runs it; it prints one JSON line per measurement.
 * A 4000-commit, ~22-lane generated repo with a picture for every author (answered in the
 * WebSocket, the harness has none), a 2560×1440 viewport at dpr 2, the Graph column at its
 * widest (every lane fits). Measures:
 * - every drawGraph's duration (the e2e build's `window.__gbGraphDraws` probe) over jumps to 60
 *   scroll offsets, once the authors' pictures are cached: the band redraw;
 * - a ~2 s rAF-driven scroll and a ~2 s mouse-wheel scroll: frames over 16.7 ms, long tasks,
 *   React commits per second (a stub devtools hook counts them), draws and their durations.
 */

const stats = (xs: number[]) => {
  const s = [...xs].sort((a, b) => a - b);
  const q = (p: number) => (s.length ? s[Math.min(s.length - 1, Math.floor(p * s.length))] : 0);
  return { n: s.length, median: +q(0.5).toFixed(2), p95: +q(0.95).toFixed(2), max: +(s.at(-1) ?? 0).toFixed(2) };
};

test.describe('graph scroll perf probe', () => {
  test.skip(!process.env.GITVOLT_PERF, 'the graph perf probe runs with GITVOLT_PERF=1');
  test.use({ viewport: { width: 2560, height: 1440 }, deviceScaleFactor: 2 });
  test.setTimeout(120_000);

  test('graph scroll perf probe', async ({ page }) => {
    const repo = makePerfRepo(join(mkdtempSync(join(fixtures.notRepo, 'perf-')), 'big'));
    await page.addInitScript(() => {
      const w = window as unknown as { __gbCommits: number; __REACT_DEVTOOLS_GLOBAL_HOOK__: object };
      w.__gbCommits = 0;
      w.__REACT_DEVTOOLS_GLOBAL_HOOK__ = { isDisabled: false, supportsFiber: true, renderers: new Map(), inject: () => 1, checkDCE() {}, onScheduleFiberRoot() {}, onCommitFiberUnmount() {}, onPostCommitFiberRoot() {}, onCommitFiberRoot: () => { w.__gbCommits++; } };
    });
    let pictures = 0;
    await page.routeWebSocket(/\/ws$/, (ws) => {
      const server = ws.connectToServer();
      ws.onMessage((m) => {
        const msg = JSON.parse(String(m)) as { id: number; req?: { method: string; params: { email: string } } };
        if (msg.req?.method !== 'avatar') return server.send(m);
        pictures++;
        ws.send(JSON.stringify({ id: msg.id, ok: { mime: 'image/png', base64: avatarPng(msg.req.params.email) } }));
      });
    });
    await page.goto(openUrl(repo));
    const grid = page.getByRole('grid', { name: 'Commit graph' });
    await expect(grid).toBeVisible();
    await expect(grid.getByRole('row').nth(30)).toBeVisible();
    const info = await page.evaluate(() => {
      const g = document.querySelector<HTMLElement>('[role="grid"][aria-label="Commit graph"]')!;
      const c = document.querySelector<HTMLCanvasElement>('[data-testid="graph-canvas"]')!;
      return { rows: Number(g.getAttribute('aria-rowcount')), viewportH: g.clientHeight, scrollH: g.scrollHeight, canvasCss: [c.getBoundingClientRect().width, c.getBoundingClientRect().height], backing: [c.width, c.height], dpr: devicePixelRatio };
    });
    console.log(JSON.stringify({ probe: 'setup', ...info }));

    // The band redraw: jumps to 60 offsets, twice (the first pass loads the pictures).
    const jumps = await page.evaluate(async () => {
      const g = document.querySelector<HTMLElement>('[role="grid"][aria-label="Commit graph"]')!;
      const frame = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
      const max = Math.min(g.scrollHeight - g.clientHeight, 60_000);
      const offsets = Array.from({ length: 60 }, (_, i) => Math.round(((i * 7919) % 60) / 60 * max) + 13);
      for (const y of offsets) { g.scrollTop = y; await frame(); }
      await new Promise((r) => setTimeout(r, 1500));
      for (const y of offsets) { g.scrollTop = y; await frame(); }
      await new Promise((r) => setTimeout(r, 1500));
      window.__gbGraphDraws = [];
      for (const y of offsets) { g.scrollTop = y; await frame(); }
      const draws = window.__gbGraphDraws;
      window.__gbGraphDraws = undefined;
      return draws;
    });
    console.log(JSON.stringify({ probe: 'jump redraws (pictures cached)', picturesServed: pictures, draws: stats(jumps) }));

    // A continuous scroll: `drive` moves it for ~2 s while the page records frames.
    const measure = async (label: string, drive: () => Promise<void>) => {
      await page.evaluate(() => {
        const g = document.querySelector<HTMLElement>('[role="grid"][aria-label="Commit graph"]')!;
        g.scrollTop = 0;
      });
      await page.waitForTimeout(500);
      await page.evaluate(() => {
        const w = window as unknown as { __gbProbe: { deltas: number[]; counts: number[]; long: number[]; commits0: number; t0: number; stop: boolean; po: PerformanceObserver }; __gbCommits: number };
        const long: number[] = [];
        const po = new PerformanceObserver((l) => { for (const e of l.getEntries()) long.push(e.duration); });
        po.observe({ type: 'longtask' });
        w.__gbProbe = { deltas: [], counts: [], long, commits0: w.__gbCommits, t0: performance.now(), stop: false, po };
        window.__gbGraphDraws = [];
        let last: number | undefined;
        const tick = (t: number) => {
          if (last !== undefined) w.__gbProbe.deltas.push(t - last);
          w.__gbProbe.counts.push(window.__gbGraphDraws?.length ?? 0);
          last = t;
          if (!w.__gbProbe.stop) requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
      });
      await drive();
      const r = await page.evaluate(() => {
        const w = window as unknown as { __gbProbe: { deltas: number[]; counts: number[]; long: number[]; commits0: number; t0: number; stop: boolean; po: PerformanceObserver }; __gbCommits: number };
        const p = w.__gbProbe;
        p.stop = true;
        p.po.disconnect();
        const secs = (performance.now() - p.t0) / 1000;
        const draws = window.__gbGraphDraws ?? [];
        window.__gbGraphDraws = undefined;
        const g = document.querySelector<HTMLElement>('[role="grid"][aria-label="Commit graph"]')!;
        return { secs, scrolled: g.scrollTop, deltas: p.deltas, counts: p.counts, long: p.long, commits: w.__gbCommits - p.commits0, draws };
      });
      console.log(JSON.stringify({
        probe: label, secs: +r.secs.toFixed(2), scrolledPx: Math.round(r.scrolled), frames: r.deltas.length,
        // Over one 16.7 ms frame, past vsync jitter (deltas read 16.6–16.8 ms at 60 Hz): a missed frame.
        framesOver16_7: r.deltas.filter((d) => d > 18).length, framesOver33: r.deltas.filter((d) => d > 33.4).length, frameMs: stats(r.deltas),
        // Of those, the ones right after a frame that drew the canvas (its paint carried the draw).
        framesOver16_7AfterADraw: r.deltas.filter((d, i) => d > 18 && r.counts[i] > (r.counts[i - 1] ?? 0)).length,
        longTasks: r.long.length, longTaskMs: Math.round(r.long.reduce((a, b) => a + b, 0)),
        reactCommits: r.commits, reactCommitsPerSec: +(r.commits / r.secs).toFixed(1),
        draws: stats(r.draws), drawsPerSec: +(r.draws.length / r.secs).toFixed(1),
      }));
    };

    for (const px of [24, 6]) {
      await measure(`rAF scroll, ${px} px/frame`, () => page.evaluate(async (dy) => {
        const g = document.querySelector<HTMLElement>('[role="grid"][aria-label="Commit graph"]')!;
        const t0 = performance.now();
        await new Promise<void>((done) => {
          const step = () => {
            g.scrollTop += dy;
            if (performance.now() - t0 < 2000) requestAnimationFrame(step);
            else done();
          };
          requestAnimationFrame(step);
        });
      }, px));
    }

    const box = (await grid.boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await measure('mouse wheel, 100 px every ~16 ms', async () => {
      const t0 = Date.now();
      while (Date.now() - t0 < 2000) {
        await page.mouse.wheel(0, 100);
        await page.waitForTimeout(16);
      }
    });
  });
});

/** A preset switch on the perf repo (its window: the 2000-commit default) re-lays out the walked
 * commits, no new walk. Most of the time is the debug harness building and sending the graph
 * (about 1 MB of rows): reloading All takes as long, and its times are logged beside. */
const SWITCH_MS = 300;

test(`switching the 4 000-commit repo to "All, collapsed" shows the new graph in under ${SWITCH_MS} ms (best of a few)`, { tag: '@budget' }, async ({ page }) => {
  const repo = makePerfRepo(join(mkdtempSync(join(fixtures.notRepo, 'perf-')), 'big'));
  await page.goto(openUrl(repo));
  const grid = page.getByRole('grid', { name: 'Commit graph' });
  await expect(grid.getByRole('row').nth(20)).toBeVisible();
  const all = await grid.getAttribute('aria-rowcount');
  const button = page.locator('.preset-button');
  /** From a click on `selector` to the first frame painted after the grid took its new rows. */
  const timed = (selector: string) => page.evaluate((sel) => new Promise<number>((resolve) => {
    const g = document.querySelector<HTMLElement>('[role="grid"][aria-label="Commit graph"]')!;
    const t0 = performance.now();
    new MutationObserver((_, o) => {
      o.disconnect();
      requestAnimationFrame(() => requestAnimationFrame(() => resolve(performance.now() - t0)));
    }).observe(g, { attributes: true, attributeFilter: ['aria-rowcount'] });
    document.querySelector<HTMLElement>(sel)!.click();
  }), selector);
  const times: number[] = [];
  const back: number[] = [];
  for (let i = 0; i < 5 && !(i >= 3 && Math.min(...times) < SWITCH_MS); i++) {
    times.push(await timed('.preset-main'));
    await expect(button).toHaveAttribute('data-preset', 'all-collapsed');
    expect(Number(await grid.getAttribute('aria-rowcount'))).toBeLessThan(Number(all));
    await button.getByRole('button', { name: 'All branch presets' }).click();
    back.push(await timed('[data-row-id="preset.all"]'));
    await expect(grid).toHaveAttribute('aria-rowcount', all!);
  }
  const ms = (xs: number[]) => xs.map((t) => t.toFixed(1)).join(', ');
  console.log(`[budget] switch to All, collapsed (4 000 commits): ${ms(times)} ms; back to All: ${ms(back)} ms`);
  if (budgetApplies()) expect(Math.min(...times), `switch times ${ms(times)} ms`).toBeLessThan(SWITCH_MS);
});
