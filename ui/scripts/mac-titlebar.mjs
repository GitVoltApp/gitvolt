// GitVolt's own title bar on macOS, checked on a running app with real input: the page over the
// DevTools protocol (`GITVOLT_DEV_CDP_PORT`, debug builds) for what it draws, and `mac-window`
// (built from mac-window.swift) for the window server's view of the window, the menu bar, and
// mouse and key events posted to the system. Each step leaves a screenshot of the whole screen in
// <out dir>. With `none` for the port (a release build, which refuses CDP), only what the window
// server and Accessibility show is checked.
//
//   node ui/scripts/mac-titlebar.mjs <cdp port|none> <out dir> <mac-window> <app pid> <app executable>
//
// The steps: the layout (the traffic lights' slot, ☰, the title, no buttons at the end) and where
// the traffic lights are; a resize from the frame's corner (macOS keeps its frame); a drag of the
// title bar moves the window, and leaves no button held in the page; a click on ☰ up there opens
// the menu; a double click zooms and a second one unzooms; System Settings' double click action
// (Minimize, then None) changed while the app runs, read again when the window comes back to the
// front; the yellow light minimizes (a second launch brings it back), the green one enters full
// screen, where the slot goes, and Ctrl+Cmd+F (Window › Exit Full Screen) leaves it; Ctrl+Cmd+F
// enters and leaves full screen again; the red one closes the window. Exits 1 if a step failed,
// after all ran.
import { execFile } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { chromium } from '@playwright/test';

const [port, outDir, tool, pidArg, appExe] = process.argv.slice(2);
if (!port || !outDir || !tool || !pidArg || !appExe) {
  console.error('usage: mac-titlebar.mjs <cdp port|none> <out dir> <mac-window> <app pid> <app executable>');
  process.exit(2);
}
const pid = Number(pidArg);
mkdirSync(outDir, { recursive: true });

const results = [];
let shot = 0;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function run(file, args) {
  return new Promise((resolve, reject) => {
    execFile(file, args, { timeout: 60_000 }, (error, stdout, stderr) =>
      error ? reject(new Error(`${file} ${args.join(' ')} failed (${error.code ?? error.signal}): ${stderr.slice(-500)}`)) : resolve(stdout));
  });
}
const mac = async (...args) => {
  const out = (await run(tool, args.map(String))).trim();
  return out ? JSON.parse(out) : null;
};
/** The app's window as the window server has it: the largest of the pid's normal windows. */
async function frame() {
  const windows = await mac('windows', pid);
  const main = windows.sort((a, b) => b.bounds.w * b.bounds.h - a.bounds.w * a.bounds.h)[0];
  if (!main) throw new Error('the app has no window');
  return { ...main.bounds, onscreen: main.onscreen };
}
const near = (a, b, tolerance) => Math.abs(a - b) <= tolerance;
const fmt = (f) => `${Math.round(f.x)},${Math.round(f.y)} ${Math.round(f.w)}x${Math.round(f.h)}`;

async function step(name, body) {
  const started = Date.now();
  const notes = [];
  try {
    await body((note) => notes.push(note));
    results.push({ step: name, ok: true, ms: Date.now() - started, notes });
  } catch (e) {
    results.push({ step: name, ok: false, ms: Date.now() - started, notes, error: String(e?.message ?? e).split('\n')[0] });
  }
  shot += 1;
  await run('screencapture', ['-x', join(outDir, `${String(shot).padStart(2, '0')}-${name}.png`)]).catch(() => {});
}

const browser = port === 'none' ? null : await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
const page = browser?.contexts().flatMap((c) => c.pages()).find((p) => !p.url().startsWith('devtools:'));
if (browser && !page) throw new Error('no app page');
const bar = page?.locator('.title-bar');
/** The traffic lights' slot in the page's bar (`.title-bar-lights`), for a check without the page. */
const SLOT_WIDTH = 78;
const rectOf = (locator) => locator.evaluate((el) => { const r = el.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; });
/** A point of the page, in screen coordinates (the page fills the window, at 100% zoom). */
const onScreen = async (x, y) => { const f = await frame(); return [f.x + x, f.y + y]; };
/** A point on the title bar's empty space: three quarters of the way across. */
const barSpace = async () => { const f = await frame(); return [f.x + f.w * 0.75, f.y + 15]; };
let lights = null;

await step('layout', async (note) => {
  if (!page) {
    const f = await frame();
    lights = await mac('lights', pid);
    note(`no page (a release build); window ${fmt(f)}; Accessibility ${JSON.stringify(lights)}`);
    if (!lights.close) throw new Error('no traffic lights through Accessibility');
    if (lights.zoom.x + lights.zoom.w - f.x > SLOT_WIDTH) throw new Error('the traffic lights overflow their slot');
    if (!near(lights.close.y + lights.close.h / 2 - f.y, 15, 1)) throw new Error('the traffic lights are not centred in the bar');
    return;
  }
  const info = await page.evaluate(() => window.__GITVOLT_WINDOW__);
  note(`__GITVOLT_WINDOW__ ${JSON.stringify(info)}`);
  if (info?.frame !== 'custom' || info?.platform !== 'macos') throw new Error('not the custom macOS title bar');
  await bar.waitFor({ timeout: 30_000 });
  const order = await bar.evaluate((el) => [...el.children].map((c) => c.getAttribute('aria-label') ?? c.className));
  note(`bar children ${JSON.stringify(order)}`);
  if (JSON.stringify(order) !== JSON.stringify(['title-bar-lights', 'Menu', 'title-bar-title'])) throw new Error(`unexpected bar ${JSON.stringify(order)}`);
  const [b, slot, menu, tabs] = await Promise.all([rectOf(bar), rectOf(bar.locator('.title-bar-lights')), rectOf(bar.getByRole('button', { name: 'Menu' })), rectOf(page.locator('.tab-bar'))]);
  note(`bar ${JSON.stringify(b)} slot ${JSON.stringify(slot)} menu ${JSON.stringify(menu)} tab bar y ${tabs.y}`);
  if (b.y !== 0 || b.h !== 30 || tabs.y !== 30 || slot.x !== 0 || menu.x !== slot.w) throw new Error('the bar is not 30 px at the top, with ☰ after the slot');
  if (await page.locator('.window-edges').count()) throw new Error('resize edges on macOS');
  const f = await frame();
  lights = await mac('lights', pid);
  note(`window ${fmt(f)}; Accessibility ${JSON.stringify(lights)}`);
  if (lights.close) {
    for (const name of ['close', 'minimize', 'zoom']) {
      const l = lights[name];
      const centre = l.y + l.h / 2 - f.y;
      note(`${name}: ${l.x - f.x},${l.y - f.y} ${l.w}x${l.h}, centre ${centre} px down (the bar's is 15)`);
    }
    const zoom = lights.zoom;
    if (zoom.x + zoom.w - f.x > slot.w) throw new Error('the traffic lights overflow their slot');
    if (!near(lights.close.y + lights.close.h / 2 - f.y, 15, 1)) throw new Error('the traffic lights are not centred in the bar');
  }
});

await step('resize-from-corner', async (note) => {
  const before = await frame();
  await mac('drag', before.x + before.w - 2, before.y + before.h - 2, before.x + before.w - 140, before.y + before.h - 170);
  await sleep(800);
  const after = await frame();
  note(`${fmt(before)} -> ${fmt(after)}`);
  if (!(after.w < before.w - 20 || after.h < before.h - 20)) {
    // The later steps want room to zoom into: the window made smaller another way.
    note(`through Accessibility instead: ${JSON.stringify(await mac('size', pid, 900, 520))}`);
    await sleep(800);
    note(`now ${fmt(await frame())}`);
    throw new Error("the frame's corner didn't resize the window");
  }
});

await step('drag-title-bar', async (note) => {
  await page?.evaluate(() => {
    window.__tbButtons = [];
    document.addEventListener('mousemove', (e) => window.__tbButtons.push(e.buttons), { capture: true });
  });
  // Twice: the window starts against the screen's left edge, which holds a window back a little
  // as it leaves; the second drag is from clear of it.
  let before, after, dx, dy;
  for (const [tx, ty] of [[90, 60], [60, 40]]) {
    before = await frame();
    const [x, y] = await barSpace();
    await mac('drag', x, y, x + tx, y + ty);
    await sleep(800);
    after = await frame();
    [dx, dy] = [after.x - before.x, after.y - before.y];
    // Where the press was in the window, and where the pointer ended up in it: the same point
    // when the window followed the pointer all the way.
    note(`${fmt(before)} -> ${fmt(after)}: moved ${dx},${dy} for a pointer travel of ${tx},${ty}; grab ${Math.round(x - before.x)},${Math.round(y - before.y)} -> ${Math.round(x + tx - after.x)},${Math.round(y + ty - after.y)}`);
  }
  if (!(dx > 30 && dx <= 65 && dy > 20 && dy <= 45)) throw new Error(`the window moved ${dx},${dy}`);
  if (after.w !== before.w || after.h !== before.h) throw new Error('the drag resized the window');
  if (!page) return;
  // The page saw the press; after the window server's drag it must not think the button is
  // still down.
  await page.evaluate(() => { window.__tbButtons = []; });
  const [mx, my] = await onScreen(after.w / 2, after.h / 2);
  await mac('move', mx, my);
  await sleep(150);
  await mac('move', mx + 20, my + 10);
  await sleep(300);
  const buttons = await page.evaluate(() => window.__tbButtons);
  note(`mousemove buttons after the drag: ${JSON.stringify(buttons)}`);
  if (buttons.some((b) => b !== 0)) throw new Error('the page thinks a button is still held');
});

if (page) await step('menu-in-title-bar', async () => {
  const menu = await rectOf(bar.getByRole('button', { name: 'Menu' }));
  const [x, y] = await onScreen(menu.x + menu.w / 2, menu.y + menu.h / 2);
  await mac('click', x, y);
  await page.getByRole('menu', { name: 'Menu' }).waitFor({ timeout: 5_000 });
  await page.keyboard.press('Escape');
});

await step('double-click-zooms', async (note) => {
  const before = await frame();
  const screen = await mac('screen');
  note(`screen ${JSON.stringify(screen)}`);
  await mac('click', ...(await barSpace()), 2);
  await sleep(1500);
  const zoomed = await frame();
  note(`${fmt(before)} -> ${fmt(zoomed)}`);
  if (!(zoomed.w > before.w + 20 || zoomed.h > before.h + 20)) throw new Error("a double click didn't zoom");
  await mac('click', ...(await barSpace()), 2);
  await sleep(1500);
  const back = await frame();
  note(`double click again -> ${fmt(back)}`);
  if (!(near(back.w, before.w, 4) && near(back.h, before.h, 4))) throw new Error("a second double click didn't unzoom");
});

/** The window goes to the back and comes to the front again: another app opens in front of it
 * and quits. (Not a click on the desktop: since macOS 14 that moves every window aside.) */
async function refocus() {
  await run('open', ['-a', 'Calculator']);
  await sleep(2500);
  await run('pkill', ['-x', 'Calculator']).catch(() => {});
  await sleep(2000);
}

await step('double-click-setting', async (note) => {
  const read = await run('defaults', ['read', '-g', 'AppleActionOnDoubleClick']).catch(() => null);
  const original = read?.trim() || null;
  note(`AppleActionOnDoubleClick was ${original ?? 'unset'}`);
  const failures = [];
  try {
    for (const value of ['Minimize', 'None']) {
      await run('defaults', ['write', '-g', 'AppleActionOnDoubleClick', '-string', value]);
      await refocus();
      const before = await frame();
      await mac('click', ...(await barSpace()), 2);
      await sleep(1500);
      const after = await frame();
      note(`${value}: ${fmt(before)} -> ${fmt(after)}, on screen ${after.onscreen}`);
      await run('screencapture', ['-x', join(outDir, `double-click-${value}.png`)]).catch(() => {});
      if (value === 'Minimize') {
        if (after.onscreen) failures.push("with Minimize, a double click didn't minimize");
        await run(appExe, []);
        await sleep(2000);
        if (!(await frame()).onscreen) failures.push("the window didn't come back");
      } else if (!after.onscreen || after.x !== before.x || after.y !== before.y || after.w !== before.w || after.h !== before.h) {
        failures.push('with None, a double click changed the window');
      }
    }
  } finally {
    await run('defaults', original ? ['write', '-g', 'AppleActionOnDoubleClick', '-string', original] : ['delete', '-g', 'AppleActionOnDoubleClick']).catch(() => {});
    if ((await frame()).onscreen) await refocus();
  }
  if (failures.length) throw new Error(failures.join('; '));
});

/** The Window menu's full screen item as Accessibility has it (its title, key and modifiers). */
async function fullScreenItem() {
  const menu = await mac('menu', pid, 'Window').catch((e) => ({ error: String(e.message) }));
  const item = menu?.items?.find((i) => /^(Enter|Exit) Full Screen$/.test(i.title));
  return item ? `"${item.title}" (key ${item.key}, modifiers ${item.modifiers}, enabled ${item.enabled})` : `none: ${JSON.stringify(menu)}`;
}

/** Whether the window fills the screen; and the page's traffic light slots (none in full screen). */
async function fullScreen() {
  const f = await frame();
  const screen = await mac('screen');
  const slot = page ? await bar.locator('.title-bar-lights').count() : null;
  return { f, screen, slot, full: near(f.w, screen.frame.w, 2) && near(f.h, screen.frame.h, 2) };
}

/** A traffic light's centre: Accessibility's frame, or where TRAFFIC_LIGHTS puts it. */
async function light(name) {
  const f = await frame();
  const now = await mac('lights', pid);
  const l = now?.[name];
  if (l) return [l.x + l.w / 2, l.y + l.h / 2];
  return [f.x + 12 + 7 + 20 * ['close', 'minimize', 'zoom'].indexOf(name), f.y + 15];
}

await step('yellow-minimizes', async (note) => {
  await mac('click', ...(await light('minimize')));
  await sleep(1500);
  const minimized = await frame();
  note(`after the yellow light: on screen ${minimized.onscreen}`);
  if (minimized.onscreen) throw new Error("the yellow light didn't minimize");
  // A second launch brings the window back (lib.rs `bring_to_front`).
  await run(appExe, []);
  await sleep(2000);
  const back = await frame();
  note(`after a second launch: on screen ${back.onscreen}`);
  if (!back.onscreen) throw new Error("the window didn't come back");
});

await step('green-full-screen', async (note) => {
  await mac('click', ...(await light('zoom')));
  await sleep(3000);
  note(`Window menu: ${await fullScreenItem()}`);
  if (!page) {
    const { f, screen, full } = await fullScreen();
    note(`window ${fmt(f)}, screen ${JSON.stringify(screen.frame)}`);
    if (!full) throw new Error('the green light did not make the window full screen');
    return;
  }
  const f = await frame();
  const screen = await mac('screen');
  const slot = await bar.locator('.title-bar-lights').count();
  const [b, menu] = await Promise.all([rectOf(bar), rectOf(bar.getByRole('button', { name: 'Menu' }))]);
  note(`window ${fmt(f)}, screen ${JSON.stringify(screen.frame)}; slot ${slot}, bar ${JSON.stringify(b)}, ☰ x ${menu.x}`);
  if (!(near(f.w, screen.frame.w, 2) && near(f.h, screen.frame.h, 2))) throw new Error('the green light did not make the window full screen');
  if (slot !== 0 || menu.x !== 0 || b.y !== 0) throw new Error('the bar kept the traffic lights\' slot in full screen');
  await page.screenshot({ path: join(outDir, 'page-full-screen.png') }).catch(() => {});
});

await step('leave-full-screen', async (note) => {
  // Ctrl+Cmd+F: the Window menu's Exit Full Screen.
  await mac('key', 3, 'ctrl', 'cmd');
  await sleep(3000);
  const keyed = !(await fullScreen()).full;
  if (!keyed) {
    note('Ctrl+Cmd+F left it in full screen; asking through Accessibility');
    note(JSON.stringify(await mac('fullscreen', pid, 0)));
    await sleep(3000);
  }
  const { f, slot, full } = await fullScreen();
  note(`window ${fmt(f)}; slot ${slot}; Window menu: ${await fullScreenItem()}`);
  if (full) throw new Error('the window stayed in full screen');
  if (page && slot !== 1) throw new Error('the slot did not come back after full screen');
  const after = await mac('lights', pid);
  if (after.close) note(`close light at ${after.close.x - f.x},${after.close.y - f.y}`);
  if (!keyed) throw new Error("Ctrl+Cmd+F didn't leave full screen");
});

await step('full-screen-keys', async (note) => {
  const before = await frame();
  // A click on the title bar first: the pointer, still where the green light was, opened its
  // hover menu (Full Screen ›, Tile…), and that panel would take the key.
  await mac('click', ...(await barSpace()));
  await sleep(1000);
  await mac('key', 3, 'ctrl', 'cmd');
  await sleep(3000);
  const into = await fullScreen();
  note(`Ctrl+Cmd+F: window ${fmt(into.f)}, slot ${into.slot}; Window menu: ${await fullScreenItem()}`);
  await run('screencapture', ['-x', join(outDir, 'full-screen-by-keys.png')]).catch(() => {});
  await mac('key', 3, 'ctrl', 'cmd');
  await sleep(3000);
  const back = await fullScreen();
  note(`Ctrl+Cmd+F again: window ${fmt(back.f)} (was ${fmt(before)}), slot ${back.slot}; Window menu: ${await fullScreenItem()}`);
  if (!into.full || (page && into.slot !== 0)) throw new Error("Ctrl+Cmd+F didn't enter full screen");
  if (back.full || (page && back.slot !== 1)) throw new Error("Ctrl+Cmd+F didn't leave full screen");
});

await step('red-closes', async (note) => {
  await mac('click', ...(await light('close')));
  await sleep(4000);
  let alive = true;
  try { process.kill(pid, 0); } catch { alive = false; }
  const windows = await mac('windows', pid).catch(() => []);
  note(`process alive ${alive}; windows ${JSON.stringify(windows)}`);
  if (alive && windows.some((w) => w.onscreen)) throw new Error("the red light didn't close the window");
});

writeFileSync(join(outDir, 'results.json'), JSON.stringify(results, null, 2));
for (const r of results) {
  console.log(`${r.ok ? 'ok  ' : 'FAIL'} ${r.step} (${r.ms} ms)${r.error ? `: ${r.error}` : ''}`);
  for (const n of r.notes) console.log(`       ${n}`);
}
await browser?.close().catch(() => {});
process.exit(results.every((r) => r.ok) ? 0 : 1);
