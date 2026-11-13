import { api } from '../api/client';
import { useActivityUi } from '../app/activityLog';
import { useOps } from '../app/ops';
import { useAppState } from '../app/state';
import { tabStore } from '../app/tabStores';
import { centerViewOf } from '../repo/centerView';

/**
 * The busy-page detector: when the page keeps the main thread busy with nothing to show for it
 * (a `requestAnimationFrame` loop that never stops, long animation frames back to back, an endless
 * CSS animation that restyles every frame), one warning line goes to the local log file with what
 * was running and from where. A profiler can't be attached to a release build; this is the
 * evidence for the next time it happens.
 *
 * It costs nothing while the page is idle: no timer or loop of its own. It counts rAF requests as
 * they're made, and Chromium tells it about long animation frames and animation iterations. The
 * one timer, which waits for an episode to end, exists only during one.
 */
export interface BusyThresholds {
  /** How long a source must stay busy before it's reported. */
  sustainMs: number;
  /** rAF: a pause longer than this between requests ends a run. */
  rafGapMs: number;
  /** rAF: the run's average rate, in requests per second (60 for a loop at 60 Hz). */
  rafMinPerSec: number;
  /** Long animation frames: their share of the run's time. */
  loafShare: number;
  /** Long animation frames: a pause longer than this between two ends a run. */
  loafGapMs: number;
  /** CSS animations: a pause longer than this between two iterations ends a run. */
  iterGapMs: number;
  /** An episode ends when its sources have been quiet this long. */
  quietMs: number;
  /** At most one report per this long; a busy page still going after it is reported again. */
  reportEveryMs: number;
  /** One rAF request in this many has its call site sampled (a stack trace). */
  sampleEvery: number;
}

/**
 * A loop at 60 Hz for 10 s, or long frames filling half of 10 s, is past any transition or
 * scroll the UI animates (a few hundred ms), and well short of what the user saw (minutes).
 */
export const BUSY_THRESHOLDS: BusyThresholds = {
  sustainMs: 10_000,
  rafGapMs: 100,
  rafMinPerSec: 20,
  loafShare: 0.5,
  loafGapMs: 1_000,
  iterGapMs: 5_000,
  quietMs: 2_000,
  reportEveryMs: 5 * 60_000,
  sampleEvery: 16,
};

type Why = 'raf' | 'loaf' | 'css';
type Log = (level: 'warn', message: string, stack: string | null) => Promise<unknown>;

export interface BusyPageDetector {
  configure: (patch: Partial<BusyThresholds>) => void;
  uninstall: () => void;
}

interface Options {
  log?: Log;
  now?: () => number;
  thresholds?: Partial<BusyThresholds>;
  /** Whether something legitimately animates: an op running (its spinner), the perf overlay (its frame meter). */
  expected?: () => boolean;
  /** The app state the report names (tabs, view): kept apart so the tests needn't build it. */
  appState?: () => AppSnapshot;
}

interface LoafScript { invoker?: string; invokerType?: string; sourceFunctionName?: string; sourceURL?: string; sourceCharPosition?: number; duration: number }
interface LongFrame extends PerformanceEntry { styleAndLayoutStart?: number; scripts?: LoafScript[] }

export interface AppSnapshot { tabs: number; view: string | null; diffOpen: boolean; ops: number }

const TALLY_CAP = 32;
const KEY_MAX = 160;
const MESSAGE_MAX = 8 * 1024;
const COMPOSITED = new Set(['transform', 'opacity', 'translate', 'rotate', 'scale']);
const NOT_PROPS = new Set(['offset', 'computedOffset', 'easing', 'composite']);

/** Counts per key, at most `TALLY_CAP` keys: the rest add up under `<other>`. */
class Tally {
  private m = new Map<string, number>();
  add(key: string, n = 1) {
    const k = this.m.has(key) || this.m.size < TALLY_CAP ? key : '<other>';
    this.m.set(k, (this.m.get(k) ?? 0) + n);
  }
  top(n: number): [string, number][] {
    return [...this.m].sort((a, b) => b[1] - a[1]).slice(0, n).map(([k, v]) => [k, Math.round(v)]);
  }
  clear() { this.m.clear(); }
}

/** A script URL as its path when it's the app's own (a bundle chunk), else `<other>`. */
export function codePath(url: string | undefined, origin: string): string {
  if (!url) return '';
  try {
    const u = new URL(url);
    return u.origin === origin ? u.pathname : '<other>';
  } catch {
    return '<other>';
  }
}

const cleanName = (s: string | undefined) => (s ?? '').replace(/[^\w$.<> #-]/g, '').slice(0, 60);
const clip = (s: string) => (s.length > KEY_MAX ? s.slice(0, KEY_MAX) : s);

/** One stack frame as `name path:line:col` (V8's `at name (url:l:c)` or JavaScriptCore's
 * `name@url:l:c`). Only code locations: a URL that isn't the app's own keeps its function name
 * and loses the rest. */
function frame(line: string | undefined, origin: string): string | null {
  if (!line) return null;
  const m = /^at (?:(.*?) \()?(.*?):(\d+):(\d+)\)?$/.exec(line) ?? /^(.*?)@(.*?):(\d+):(\d+)$/.exec(line);
  // No location (JavaScriptCore's `name@` for evaluated code, `name@[native code]`): the name alone.
  if (!m) return `${cleanName(line.replace(/^at /, '').split(/[@(]/)[0].trim()) || '<anon>'} <other>`;
  const [, name, url, l, c] = m;
  const path = codePath(url, origin);
  return `${cleanName(name) || '<anon>'} ${path}${path === '<other>' ? '' : `:${l}:${c}`}`;
}

/** A rAF request's call site, from a stack taken inside the wrapper: its caller, and the caller's
 * caller (a library's scheduler is called by the code that wants the frame). */
export function callSite(stack: string | undefined, origin: string): string {
  const frames = (stack ?? '').split('\n').map((l) => l.trim()).filter((l) => /^at |@/.test(l)); // not V8's `Error:` header
  const caller = frame(frames[1], origin);
  if (!caller) return '<unknown>';
  const parent = frame(frames[2], origin);
  return clip(parent ? `${caller} < ${parent}` : caller);
}

function scriptKey(s: LoafScript, origin: string): string {
  const path = codePath(s.sourceURL, origin);
  const where = path && path !== '<other>' ? `${path}@${s.sourceCharPosition ?? -1}` : path;
  const invoker = (s.invoker ?? '').startsWith('http') ? codePath(s.invoker, origin) : cleanName(s.invoker);
  return clip(`${s.invokerType ?? '?'} ${invoker} ${cleanName(s.sourceFunctionName) || '<anon>'} ${where}`.trim());
}

/** An element as `tag.class.class`: class names are the app's own, kept only when they look like one. */
export function describeElement(el: Element | null | undefined): string | null {
  if (!el) return null;
  const classes = (el.getAttribute('class') ?? '').split(/\s+/).filter((c) => /^[A-Za-z][\w-]{0,31}$/.test(c)).slice(0, 4);
  return [el.tagName.toLowerCase(), ...classes].join('.');
}

const animatedProps = (a: Animation): string[] => {
  if (typeof CSSTransition !== 'undefined' && a instanceof CSSTransition) return [a.transitionProperty];
  const effect = a.effect as KeyframeEffect | null;
  const frames = effect && typeof effect.getKeyframes === 'function' ? effect.getKeyframes() : [];
  return [...new Set(frames.flatMap((f) => Object.keys(f).filter((k) => !NOT_PROPS.has(k))))];
};
const endless = (a: Animation) => a.effect?.getComputedTiming().endTime === Infinity;
/** Running endless animations of something other than transform or opacity: these restyle on the main thread every frame. */
const costly = (a: Animation) => a.playState === 'running' && endless(a) && animatedProps(a).some((p) => !COMPOSITED.has(p));
const allAnimations = (): Animation[] => (typeof document.getAnimations === 'function' ? document.getAnimations() : []);

function describeAnimation(a: Animation) {
  const name = typeof CSSAnimation !== 'undefined' && a instanceof CSSAnimation ? a.animationName : typeof CSSTransition !== 'undefined' && a instanceof CSSTransition ? a.transitionProperty : a.id || null;
  const target = (a.effect as KeyframeEffect | null)?.target ?? null;
  return { name: name ? cleanName(name) : null, type: a.constructor.name, target: describeElement(target), state: a.playState, endless: endless(a), props: animatedProps(a).slice(0, 4) };
}

function appSnapshot(): AppSnapshot {
  const tab = useAppState.getState().profile.activeTab;
  return {
    tabs: useAppState.getState().profile.tabs.length,
    view: tab ? centerViewOf(tab)?.kind ?? null : null,
    diffOpen: tab ? !!tabStore(tab)?.getState().diff : false,
    ops: Object.keys(useOps.getState().ops).length,
  };
}

const expectedAnimation = () => Object.keys(useOps.getState().ops).length > 0 || useActivityUi.getState().perfOverlay;

const chunksOf = (keys: string[]) => [...new Set(keys.flatMap((k) => k.match(/\/[^\s:@]+\.(?:js|ts|tsx)/g) ?? []))].slice(0, 6);

declare global {
  // The e2e and dev builds' handle, to shorten the thresholds in a test (main.tsx).
  interface Window { __gbBusy?: BusyPageDetector }
}

export function installBusyPageDetector(opts: Options = {}): BusyPageDetector {
  const win = window;
  const log: Log = opts.log ?? ((level, message, stack) => api.logFrontend(level, message, stack));
  const now = opts.now ?? (() => performance.now());
  const expected = opts.expected ?? expectedAnimation;
  const app = opts.appState ?? appSnapshot;
  const th: BusyThresholds = { ...BUSY_THRESHOLDS, ...opts.thresholds };
  const origin = win.location.origin;

  const raf = { start: -Infinity, last: -Infinity, n: 0, sampled: 0 };
  const sites = new Tally();
  const loaf = { start: -Infinity, last: -Infinity, busy: 0, layout: 0, frames: 0 };
  const scripts = new Tally();
  const css = { start: -Infinity, last: -Infinity, checkedAt: -Infinity };
  let resizeLoops = 0;
  let lastReport = -Infinity;
  let episode: { start: number; why: Why; raf: number; loafMs: number } | null = null;
  let endTimer: ReturnType<typeof setTimeout> | null = null;

  const resetRuns = (t: number) => {
    raf.start = t;
    raf.n = 0;
    sites.clear();
    loaf.start = -Infinity;
    loaf.last = -Infinity;
    css.start = t;
    css.checkedAt = t;
  };

  const report = (why: Why, start: number, t: number) => {
    const forS = (t - start) / 1000;
    // Only the sources still going: a loop that stopped a while ago isn't this episode's.
    const rafLive = t - raf.last <= th.rafGapMs;
    const loafLive = t - loaf.last <= th.sustainMs;
    const rafSites = rafLive ? sites.top(5) : [];
    const loafScripts = loafLive ? scripts.top(5) : [];
    const anims = allAnimations();
    const ranked = [...anims].sort((a, b) => Number(costly(b)) - Number(costly(a)) || Number(endless(b)) - Number(endless(a)));
    const payload = {
      why,
      forS: Math.round(forS * 10) / 10,
      raf: { perS: rafLive && t > raf.start ? Math.round(raf.n / ((t - raf.start) / 1000)) : 0, sites: rafSites },
      loaf: loafLive ? { frames: loaf.frames, busyMs: Math.round(loaf.busy), layoutMs: Math.round(loaf.layout), scripts: loafScripts } : null,
      resizeLoops,
      focus: document.hasFocus(),
      hidden: document.hidden,
      visibility: document.visibilityState,
      active: describeElement(document.activeElement),
      animations: { n: anims.length, costly: anims.filter(costly).length, top: ranked.slice(0, 3).map(describeAnimation) },
      dialogs: document.querySelectorAll('[role="dialog"]').length,
      editors: document.querySelectorAll('.monaco-editor').length,
      ...app(),
      upS: Math.round(t / 1000),
    };
    let message = `busy page: ${JSON.stringify(payload)}`;
    if (message.length > MESSAGE_MAX) message = `${message.slice(0, MESSAGE_MAX)}…`;
    const chunks = chunksOf([...rafSites, ...loafScripts].map(([k]) => k));
    // A release bundle ships no source maps and its names are minified, but the build is
    // reproducible: the version's tag built with `--sourcemap hidden` gives the same chunks, plus maps.
    const hint = `busy page: look in ${chunks.length ? chunks.join(', ') : 'the call sites above'}. rAF sites are \`name path:line:col\`, long-frame scripts \`type invoker name path@char offset\`, in this version's bundle: build its tag with \`cd ui && npx vite build --sourcemap hidden\` (the same chunks, plus .js.map files) to map them to the source`;
    void log('warn', message, null).catch(() => {});
    void log('warn', hint, null).catch(() => {});
  };

  const quiet = (t: number) => t - raf.last >= th.quietMs && t - loaf.last >= th.quietMs && (episode?.why !== 'css' || !allAnimations().some(costly));

  const checkEnd = () => {
    endTimer = null;
    if (!episode) return;
    const t = now();
    if (!quiet(t)) {
      endTimer = setTimeout(checkEnd, th.quietMs);
      return;
    }
    const end = episode.why === 'css' ? t : Math.max(raf.last, loaf.last, episode.start);
    const line = { forS: Math.round((end - episode.start) / 100) / 10, why: episode.why, rafRequests: episode.raf, loafMs: Math.round(episode.loafMs), focus: document.hasFocus(), visibility: document.visibilityState };
    void log('warn', `busy page: ended ${JSON.stringify(line)}`, null).catch(() => {});
    episode = null;
  };

  const trigger = (why: Why, start: number, t: number) => {
    if (t - lastReport < th.reportEveryMs) return;
    if (expected()) {
      resetRuns(t);
      return;
    }
    lastReport = t;
    episode = { start, why, raf: 0, loafMs: 0 };
    report(why, start, t);
    endTimer = setTimeout(checkEnd, th.quietMs);
  };

  const onRaf = () => {
    const t = now();
    if (t - raf.last > th.rafGapMs) {
      raf.start = t;
      raf.n = 0;
      sites.clear();
    }
    raf.last = t;
    raf.n++;
    if (episode) episode.raf++;
    const span = t - raf.start;
    if (!episode && span >= th.sustainMs && raf.n / (span / 1000) >= th.rafMinPerSec) trigger('raf', raf.start, t);
  };

  const onLongFrame = (e: LongFrame) => {
    if (e.startTime - loaf.last > th.loafGapMs || e.startTime - loaf.start > 3 * th.sustainMs) {
      loaf.start = e.startTime;
      loaf.busy = 0;
      loaf.layout = 0;
      loaf.frames = 0;
      scripts.clear();
    }
    const end = e.startTime + e.duration;
    loaf.last = Math.max(loaf.last, end);
    loaf.busy += e.duration;
    loaf.frames++;
    if (e.styleAndLayoutStart) loaf.layout += end - e.styleAndLayoutStart;
    for (const s of e.scripts ?? []) scripts.add(scriptKey(s, origin), s.duration);
    if (episode) episode.loafMs += e.duration;
    const span = loaf.last - loaf.start;
    if (!episode && span >= th.sustainMs && loaf.busy / span >= th.loafShare) trigger('loaf', loaf.start, now());
  };

  const onIteration = () => {
    const t = now();
    if (t - css.last > th.iterGapMs) {
      css.start = t;
      css.checkedAt = t;
    }
    css.last = t;
    if (episode || t - css.checkedAt < th.sustainMs) return;
    css.checkedAt = t;
    if (allAnimations().some(costly)) trigger('css', css.start, t);
  };

  const onError = (e: ErrorEvent) => {
    if (/ResizeObserver loop/.test(e.message ?? '')) resizeLoops++;
  };

  const native = win.requestAnimationFrame;
  // The stack is taken here, so its second frame is the caller.
  const wrapped = function requestAnimationFrame(cb: FrameRequestCallback): number {
    onRaf();
    if (++raf.sampled >= th.sampleEvery) {
      raf.sampled = 0;
      sites.add(callSite(new Error().stack, origin));
    }
    return native.call(win, cb);
  };
  win.requestAnimationFrame = wrapped;

  let observer: PerformanceObserver | null = null;
  const types: readonly string[] = typeof PerformanceObserver !== 'undefined' ? PerformanceObserver.supportedEntryTypes ?? [] : [];
  const type = types.includes('long-animation-frame') ? 'long-animation-frame' : types.includes('longtask') ? 'longtask' : null;
  if (type) {
    observer = new PerformanceObserver((list) => {
      for (const e of list.getEntries()) onLongFrame(e as LongFrame);
    });
    observer.observe({ type });
  }
  document.addEventListener('animationiteration', onIteration, true);
  win.addEventListener('error', onError);

  return {
    configure: (patch) => Object.assign(th, patch),
    uninstall: () => {
      if (win.requestAnimationFrame === wrapped) win.requestAnimationFrame = native;
      observer?.disconnect();
      document.removeEventListener('animationiteration', onIteration, true);
      win.removeEventListener('error', onError);
      if (endTimer) clearTimeout(endTimer);
    },
  };
}
