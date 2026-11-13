// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BUSY_THRESHOLDS, callSite, codePath, installBusyPageDetector, type BusyPageDetector } from './busyPage';

class FakeObserver {
  static supportedEntryTypes = ['long-animation-frame', 'longtask'];
  static last: FakeObserver | null = null;
  type: string | null = null;
  private cb: (list: { getEntries: () => PerformanceEntry[] }) => void;
  constructor(cb: (list: { getEntries: () => PerformanceEntry[] }) => void) {
    this.cb = cb;
    FakeObserver.last = this;
  }
  observe(o: { type: string }) { this.type = o.type; }
  disconnect() { this.type = null; }
  emit(entries: object[]) { this.cb({ getEntries: () => entries as PerformanceEntry[] }); }
}

let t = 0;
let native: ReturnType<typeof vi.fn>;
let log: ReturnType<typeof vi.fn<(level: string, message: string, stack: string | null) => Promise<null>>>;
let detector: BusyPageDetector | undefined;
let expected = false;
const app = () => ({ tabs: 2, view: 'history', diffOpen: false, ops: 0 });

const install = () => {
  detector = installBusyPageDetector({ log, now: () => t, expected: () => expected, appState: app });
  return detector;
};
const lines = () => log.mock.calls.map((c) => c[1] as string);
const reportOf = (line: string) => JSON.parse(line.replace(/^busy page: /, ''));

/** A rAF loop, one request per 16 ms frame, as a component that forgot to stop would make. */
function spinForever(ms: number) {
  const end = t + ms;
  while (t < end) {
    t += 16;
    window.requestAnimationFrame(() => {});
  }
}

beforeEach(() => {
  t = 0;
  expected = false;
  vi.useFakeTimers();
  native = vi.fn(() => 7);
  window.requestAnimationFrame = native as unknown as typeof window.requestAnimationFrame;
  log = vi.fn(() => Promise.resolve(null));
  FakeObserver.last = null;
  vi.stubGlobal('PerformanceObserver', FakeObserver);
});

afterEach(() => {
  detector?.uninstall();
  detector = undefined;
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('installBusyPageDetector', () => {
  it('stays out of the way: rAF is passed through, nothing runs while idle, and it uninstalls', () => {
    install();
    expect(vi.getTimerCount()).toBe(0);
    const cb = () => {};
    expect(window.requestAnimationFrame(cb)).toBe(7);
    expect(native).toHaveBeenCalledWith(cb);
    // A short burst (a transition, a drag) and a long idle: no report, no timer.
    spinForever(2_000);
    t += 60_000;
    vi.advanceTimersByTime(60_000);
    expect(vi.getTimerCount()).toBe(0);
    expect(log).not.toHaveBeenCalled();
    expect(FakeObserver.last?.type).toBe('long-animation-frame');
    detector!.uninstall();
    detector = undefined;
    expect(window.requestAnimationFrame).toBe(native);
    expect(FakeObserver.last?.type).toBeNull();
  });

  it('reports a rAF loop running for 10 s once, with a hint, then one "ended" line when it stops', () => {
    install();
    spinForever(9_900);
    expect(log).not.toHaveBeenCalled();
    spinForever(200);
    expect(log).toHaveBeenCalledTimes(2);
    const report = reportOf(lines()[0]);
    expect(report).toMatchObject({ why: 'raf', tabs: 2, view: 'history', ops: 0, visibility: 'visible', hidden: false, resizeLoops: 0, loaf: null });
    expect(report.forS).toBeGreaterThanOrEqual(10);
    expect(report.raf.perS).toBeGreaterThanOrEqual(60);
    expect(report.raf.sites[0][0]).toMatch(/^spinForever /);
    expect(report.raf.sites[0][1]).toBeGreaterThan(0);
    expect(report).toHaveProperty('focus');
    expect(report).toHaveProperty('active');
    expect(report.animations).toMatchObject({ n: expect.any(Number) });
    expect(lines()[1]).toMatch(/^busy page: look in /);
    expect(log).toHaveBeenCalledWith('warn', expect.any(String), null);

    // Still going: no more lines.
    spinForever(30_000);
    vi.advanceTimersByTime(30_000);
    expect(log).toHaveBeenCalledTimes(2);

    // Stopped: one line once it's been quiet, and the timer is gone.
    t += 5_000;
    vi.advanceTimersByTime(5_000);
    expect(log).toHaveBeenCalledTimes(3);
    const ended = JSON.parse(lines()[2].replace(/^busy page: ended /, ''));
    expect(ended).toMatchObject({ why: 'raf', visibility: 'visible' });
    expect(ended.forS).toBeGreaterThanOrEqual(40);
    expect(ended.rafRequests).toBeGreaterThan(1_800);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('reports at most once per 5 minutes', () => {
    install();
    spinForever(11_000);
    t += 5_000;
    vi.advanceTimersByTime(5_000);
    expect(log).toHaveBeenCalledTimes(3);
    t += 60_000;
    spinForever(11_000);
    t += 5_000;
    vi.advanceTimersByTime(5_000);
    expect(log).toHaveBeenCalledTimes(3);
    t += BUSY_THRESHOLDS.reportEveryMs;
    spinForever(11_000);
    expect(log).toHaveBeenCalledTimes(5);
  });

  it('says nothing while an op runs (or the perf overlay is on): a loop still going after it is reported', () => {
    install();
    expected = true;
    spinForever(15_000);
    expect(log).not.toHaveBeenCalled();
    expected = false;
    spinForever(1_000);
    expect(log).not.toHaveBeenCalled();
    spinForever(10_000);
    expect(log).toHaveBeenCalledTimes(2);
  });

  it('a slow poll is no loop: requests 200 ms apart', () => {
    install();
    for (let i = 0; i < 100; i++) {
      t += 200;
      window.requestAnimationFrame(() => {});
    }
    expect(log).not.toHaveBeenCalled();
  });

  it('reports long animation frames filling half the time, naming their scripts', () => {
    install();
    const origin = window.location.origin;
    for (let i = 0; i < 110; i++) {
      t = i * 100 + 70;
      FakeObserver.last!.emit([{
        entryType: 'long-animation-frame', startTime: i * 100, duration: 70, styleAndLayoutStart: i * 100 + 60,
        scripts: [
          { invoker: 'ResizeObserver callback', invokerType: 'user-callback', sourceFunctionName: 'measure', sourceURL: `${origin}/assets/index-AbC123.js?v=1`, sourceCharPosition: 4242, duration: 50 },
          { invoker: 'https://u:secret@forge.example.com/x.js', invokerType: 'classic-script', sourceFunctionName: '', sourceURL: 'https://u:secret@forge.example.com/x.js', sourceCharPosition: 1, duration: 5 },
        ],
      }]);
    }
    expect(log).toHaveBeenCalledTimes(2);
    const report = reportOf(lines()[0]);
    expect(report.why).toBe('loaf');
    expect(report.loaf.busyMs / (report.forS * 1000)).toBeGreaterThanOrEqual(0.5);
    expect(report.loaf.layoutMs).toBeGreaterThan(0);
    expect(report.loaf.scripts[0][0]).toBe('user-callback ResizeObserver callback measure /assets/index-AbC123.js@4242');
    expect(report.raf.sites).toEqual([]);
    expect(lines()[1]).toContain('/assets/index-AbC123.js');
    expect(lines().join('\n')).not.toMatch(/secret|forge\.example/);
  });

  it('reports an endless CSS animation that restyles every frame, not a spinner that only turns', () => {
    const el = document.createElement('span');
    el.className = 'update-pill glow';
    const anim = (prop: string) => ({
      playState: 'running', id: '',
      effect: { target: el, getComputedTiming: () => ({ endTime: Infinity }), getKeyframes: () => [{ offset: 0, [prop]: 'x' }, { offset: 1, [prop]: 'y' }] },
    });
    let running = [anim('transform')];
    document.getAnimations = () => running as unknown as Animation[];
    install();
    const iterate = (ms: number) => {
      for (const end = t + ms; t < end;) {
        t += 1_000;
        document.dispatchEvent(new Event('animationiteration'));
      }
    };
    iterate(25_000);
    expect(log).not.toHaveBeenCalled();
    running = [anim('transform'), anim('boxShadow')];
    iterate(11_000);
    expect(log).toHaveBeenCalledTimes(2);
    const report = reportOf(lines()[0]);
    expect(report.why).toBe('css');
    expect(report.animations).toMatchObject({ n: 2, costly: 1 });
    expect(report.animations.top[0]).toMatchObject({ target: 'span.update-pill.glow', state: 'running', endless: true, props: ['boxShadow'] });
    running = [];
    t += 2_000;
    vi.advanceTimersByTime(2_000);
    expect(lines()[2]).toMatch(/^busy page: ended \{"forS":/);
    // @ts-expect-error: happy-dom has none of its own
    delete document.getAnimations;
  });

  it('keeps user content out: no text, paths, titles or foreign URLs, only code locations and class names', () => {
    document.title = 'feature/secret-branch — /home/ada/orbital';
    document.body.innerHTML = '<div class="commit-box"><textarea class="commit-msg spellcheck">Fix the secret thing</textarea></div><p>/home/ada/orbital</p>';
    const area = document.querySelector('textarea')!;
    area.value = 'Fix the secret thing';
    area.focus();
    install();
    window.dispatchEvent(new ErrorEvent('error', { message: 'ResizeObserver loop completed with undelivered notifications.' }));
    spinForever(11_000);
    const all = lines().join('\n');
    const report = reportOf(lines()[0]);
    expect(report.active).toBe('textarea.commit-msg.spellcheck');
    expect(report.resizeLoops).toBe(1);
    expect(all).not.toMatch(/secret|\/home\/ada|orbital|Fix the/i);
  });
});

describe('call sites', () => {
  const origin = 'http://localhost:1420';
  it("keeps the app's own paths and only the function name of anything else", () => {
    expect(codePath('http://localhost:1420/assets/Diff-x1.js?t=2#a', origin)).toBe('/assets/Diff-x1.js');
    expect(codePath('https://gitlab.example.com/group/repo.js', origin)).toBe('<other>');
    expect(codePath('/home/ada/x.js', origin)).toBe('<other>');
  });
  it('parses V8 and JavaScriptCore stacks, skipping the wrapper', () => {
    const v8 = 'Error\n    at requestAnimationFrame (http://localhost:1420/assets/index-a.js:1:100)\n    at tick (http://localhost:1420/assets/index-a.js:1:2345)\n    at http://localhost:1420/assets/Graph-b.js:3:9';
    expect(callSite(v8, origin)).toBe('tick /assets/index-a.js:1:2345 < <anon> /assets/Graph-b.js:3:9');
    const jsc = 'requestAnimationFrame@http://localhost:1420/assets/index-a.js:1:100\nloop@file:///home/ada/x.js:5:6';
    expect(callSite(jsc, origin)).toBe('loop <other>');
    expect(callSite('requestAnimationFrame@http://localhost:1420/a.js:1:1\nbusyLoop@\nglobal code@[native code]', origin)).toBe('busyLoop <other> < global code <other>');
    expect(callSite(undefined, origin)).toBe('<unknown>');
  });
});
