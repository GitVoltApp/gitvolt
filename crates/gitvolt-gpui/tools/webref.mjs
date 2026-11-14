// Drives the CEF build as the reference for a port: its screenshots, its DOM's sizes and colours.
// From the repository's root, with `target/bench/release/gitvolt` built (debug assertions on in
// gitvolt-app and gitvolt-core, which is what opens the DevTools port):
//
//   node crates/gitvolt-gpui/tools/webref.mjs <repo> <steps.mjs> <out-prefix>
//
// `steps.mjs` default-exports `async (r) => { ... }`, with `r` as below. The instance has its own
// config, cache and data folders, under target/bench/work/web-profile.
import { spawnSync as respawn } from 'node:child_process';
import { readFileSync as readCgroup } from 'node:fs';
// In a capped scope of its own (`capped.sh`), with everything it starts: a kill stays there.
if (!/\/run-[^/]*\.scope$/m.test(readCgroup('/proc/self/cgroup', 'utf8'))) {
  const again = respawn(new URL('capped.sh', import.meta.url).pathname, [process.execPath, ...process.argv.slice(1)], { stdio: 'inherit' });
  process.exit(again.status ?? 1);
}
import { spawn } from 'node:child_process';
import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const [repo, stepsFile, prefix] = process.argv.slice(2);
const bin = resolve('target/bench/release/gitvolt');
const PORT = 9333;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

rmSync(resolve('target/bench/release/chrome-sandbox'), { force: true });
const profile = resolve('target/bench/work/web-profile');
mkdirSync(profile, { recursive: true });
const app = spawn(bin, [], { stdio: 'ignore', env: { ...process.env, CHROME_DEVEL_SANDBOX: '/usr/share/GitVolt/chrome-sandbox', GITVOLT_DEV_DIRS: profile, GITVOLT_DEV_CDP_PORT: String(PORT), GITVOLT_OPEN: resolve(repo) } });

let url;
for (let i = 0; i < 400 && !url; i++) {
  try {
    url = (await (await fetch(`http://127.0.0.1:${PORT}/json`)).json()).find((t) => t.type === 'page')?.webSocketDebuggerUrl;
  } catch {}
  if (!url) await sleep(50);
}
if (!url) throw new Error('no DevTools page');
const ws = new WebSocket(url);
await new Promise((r) => { ws.onopen = r; });
let id = 0;
const waiting = new Map();
ws.onmessage = (m) => {
  const d = JSON.parse(m.data);
  if (d.id && waiting.has(d.id)) {
    waiting.get(d.id)(d);
    waiting.delete(d.id);
  }
};
const call = (method, params = {}) => new Promise((res) => { const n = ++id; waiting.set(n, res); ws.send(JSON.stringify({ id: n, method, params })); });

const r = {
  sleep,
  call,
  /** An expression's value in the page. */
  ev: async (expression) => {
    const v = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (v.result?.exceptionDetails) console.log(JSON.stringify(v.result.exceptionDetails).slice(0, 400));
    return v.result?.result?.value;
  },
  /** A real click at a point. `button`: left or right. */
  click: async (x, y, { button = 'left', clickCount = 1 } = {}) => {
    await call('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
    await call('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button, clickCount });
    await call('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button, clickCount });
  },
  hover: (x, y) => call('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y }),
  /** The centre of the first element matching a selector, whose text includes `text` if given. */
  center: async (selector, text) => r.ev(`(() => { const e = [...document.querySelectorAll(${JSON.stringify(selector)})].find((x) => ${JSON.stringify(text ?? '')} === '' || (x.innerText || x.getAttribute('aria-label') || '').includes(${JSON.stringify(text ?? '')})); if (!e) return null; const b = e.getBoundingClientRect(); return [b.x + b.width / 2, b.y + b.height / 2]; })()`),
  /** Clicks what `center` finds; says so when there's nothing. */
  clickOn: async (selector, text, options) => {
    const at = await r.center(selector, text);
    if (!at) return console.log('nothing at', selector, text);
    await r.click(at[0], at[1], options);
    await sleep(600);
  },
  key: async (key, code, modifiers = 0) => {
    await call('Input.dispatchKeyEvent', { type: 'keyDown', key, code, modifiers, windowsVirtualKeyCode: key.length === 1 ? key.toUpperCase().charCodeAt(0) : undefined });
    await call('Input.dispatchKeyEvent', { type: 'keyUp', key, code, modifiers });
  },
  /** Text into the focused field. */
  type: (text) => call('Input.insertText', { text }),
  /** A screenshot, to `<out-prefix>-<name>.png`. */
  shot: async (name) => writeFileSync(`${prefix}-${name}.png`, Buffer.from((await call('Page.captureScreenshot', { format: 'png' })).result.data, 'base64')),
  /** Position, size and the styles a port needs, of what a selector matches. */
  metrics: async (selector, limit = 8) => JSON.parse(await r.ev(`JSON.stringify([...document.querySelectorAll(${JSON.stringify(selector)})].slice(0, ${limit}).map((e) => { const b = e.getBoundingClientRect(), c = getComputedStyle(e); return { c: e.className.toString().slice(0, 50), t: (e.innerText || e.getAttribute('aria-label') || '').replace(/\\n/g, ' | ').slice(0, 60), x: b.x | 0, y: b.y | 0, w: b.width | 0, h: b.height | 0, bg: c.backgroundColor, col: c.color, font: c.fontSize + '/' + c.lineHeight + ' ' + c.fontWeight, pad: c.padding, radius: c.borderRadius, border: c.border, shadow: c.boxShadow }; }))`)),
};
for (let i = 0; i < 300; i++) {
  if ((await r.ev(`document.querySelectorAll('.graph-row').length`)) > 0) break;
  await sleep(50);
}
await sleep(1000);
let failed = false;
try {
  await (await import(pathToFileURL(resolve(stepsFile)).href)).default(r);
} catch (e) {
  failed = true;
  console.error(e);
}
ws.close();
app.kill('SIGTERM');
await sleep(1500);
process.exit(failed ? 1 : 0);
