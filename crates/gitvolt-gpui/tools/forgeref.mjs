// The web UI on the fake forge, as the e2e suite runs it: Playwright's Chromium (the engine CEF embeds)
// on `vite preview`'s e2e bundle, through the harness's WebSocket. The CEF binary has no way to point at the
// fake forge, and this needs no profile of its own: the harness's settings and tokens are temporary files.
// From the repository's root, with `tools/forge-lab.sh start` running:
//
//   node crates/gitvolt-gpui/tools/forgeref.mjs <repo> <steps.mjs> <out-prefix>
//
// `steps.mjs` default-exports `async (r) => { ... }`: `r.page` is Playwright's page, and `shot`, `metrics` and `ev`
// are as in `webref.mjs`.
import { spawn, execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const [repo, stepsFile, prefix] = process.argv.slice(2);
const ui = resolve('ui');
const { chromium } = createRequire(resolve('ui/package.json'))('@playwright/test');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const env = { ...process.env, VITE_GITVOLT_HARNESS: 'ws://127.0.0.1:7433/ws' };
execFileSync('node', ['e2e/build-ui.mjs'], { cwd: ui, env, stdio: 'inherit' });
const vite = spawn('npx', ['vite', 'preview', '--mode', 'e2e', '--outDir', 'dist-e2e', '--port', '1420', '--strictPort'], { cwd: ui, env, stdio: 'ignore', detached: true });
for (let i = 0; i < 200; i++) {
  try { await fetch('http://localhost:1420'); break; } catch { await sleep(100); }
}
const browser = await chromium.launch({ args: ['--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE localhost, EXCLUDE 127.0.0.1'] });
const context = await browser.newContext({ viewport: { width: Number(process.env.REF_W ?? 1600), height: Number(process.env.REF_H ?? 900) }, deviceScaleFactor: 1, colorScheme: 'dark', permissions: ['clipboard-read', 'clipboard-write'] });
const page = await context.newPage();
const r = {
  page, sleep,
  ev: (expression) => page.evaluate(expression),
  shot: (name) => page.screenshot({ path: `${prefix}-${name}.png` }),
  metrics: (selector, limit = 8) => page.evaluate(([s, n]) => [...document.querySelectorAll(s)].slice(0, n).map((e) => { const b = e.getBoundingClientRect(), c = getComputedStyle(e); return { c: e.className.toString().slice(0, 50), t: (e.innerText || e.getAttribute('aria-label') || '').replace(/\n/g, ' | ').slice(0, 60), x: b.x | 0, y: b.y | 0, w: b.width | 0, h: b.height | 0, bg: c.backgroundColor, col: c.color, font: `${c.fontSize}/${c.lineHeight} ${c.fontWeight}`, pad: c.padding, radius: c.borderRadius, border: c.border, shadow: c.boxShadow }; }), [selector, limit]),
};
let failed = false;
try {
  await page.goto(`http://localhost:1420/?repo=${encodeURIComponent(resolve(repo))}`);
  await page.getByRole('grid', { name: 'Commit graph' }).waitFor({ timeout: 20000 });
  await (await import(pathToFileURL(resolve(stepsFile)).href)).default(r);
} catch (e) {
  failed = true;
  console.error(e);
  await page.screenshot({ path: `${prefix}-failed.png` }).catch(() => {});
}
await browser.close();
process.kill(-vite.pid, 'SIGTERM');
process.exit(failed ? 1 : 0);
