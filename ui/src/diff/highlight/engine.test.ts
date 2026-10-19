import { describe, expect, it } from 'vitest';
import { Engine, type Grammars, type RuleStack } from './engine';
import { FOREGROUND_OFFSET, type FromWorker, type ToWorker } from './protocol';

/** A toy grammar: the state is how many `/*` are open; a line is one token, coloured 5 inside a
 * comment and 3 outside (Monaco's metadata layout). Each line costs 0.1 ms on the fake clock. */
class Depth implements RuleStack {
  readonly depth: number;
  constructor(depth: number) { this.depth = depth; }
  equals(o: RuleStack): boolean { return o instanceof Depth && o.depth === this.depth; }
}
const colour = (fg: number) => ((fg << FOREGROUND_OFFSET) | (2 << 24) | 7) >>> 0;

function setup() {
  let now = 0;
  let lines = 0;
  const tasks: Array<() => void> = [];
  const timers: Array<() => void> = [];
  const out: FromWorker[] = [];
  const g: Grammars = {
    init: async () => [],
    has: (l) => l === 'toy',
    loaded: (l) => l === 'toy',
    load: async () => ['toy'],
    grammar: () => ({
      tokenizeLine2(line, state) {
        now += 0.1;
        lines++;
        const before = (state as Depth | null)?.depth ?? 0;
        const after = Math.max(0, before + (line.split('/*').length - 1) - (line.split('*/').length - 1));
        return { tokens: Uint32Array.of(0, colour(before > 0 || after > 0 ? 5 : 3)), ruleStack: new Depth(after) };
      },
    }),
    theme: () => null,
    code: (code) => ({ lines: [[{ content: code }]] }),
  };
  const engine = new Engine(g, (m) => out.push(m), { now: () => now, yieldTo: (fn) => tasks.push(fn), later: (fn) => timers.push(fn) });
  const send = async (m: ToWorker) => { engine.handle(m); await Promise.resolve(); await Promise.resolve(); };
  /** Runs the scheduled slices until the engine is idle. */
  const drain = () => { let n = 0; while (tasks.length) { tasks.shift()!(); n++; } return n; };
  const tokens = () => out.filter((m): m is Extract<FromWorker, { kind: 'tokens' }> => m.kind === 'tokens');
  /** Line → [fg colour, exact] from every batch so far, the latest winning. */
  const shown = () => {
    const by = new Map<number, [number, boolean]>();
    for (const m of tokens()) for (let i = 0; i < m.ends.length; i++) by.set(m.from + i, [(m.data[m.ends[i] - 1] >>> FOREGROUND_OFFSET) & 0x1ff, m.exact]);
    return by;
  };
  const init = async () => { await send({ kind: 'init', themes: [] }); expect(out).toEqual([{ kind: 'ready', themes: [] }]); out.length = 0; };
  /** The grace for a first viewport runs out. */
  const grace = () => { while (timers.length) timers.shift()!(); };
  return { engine, init, grace, send, drain, out, tokens, shown, tasks, lineCount: () => lines, clear: () => { out.length = 0; lines = 0; } };
}

const open = (doc: number, text: string, known = false): ToWorker => ({ kind: 'open', doc, epoch: 0, version: 1, lang: 'toy', langBits: 9, theme: 't', text, known });
const textOf = (n: number, at: Record<number, string> = {}) => Array.from({ length: n }, (_, i) => at[i] ?? `line ${i}`).join('\n');

describe('the highlight engine', () => {
  it('tokenizes a text from the top in slices, as batches of consecutive lines in Monaco encoding, then says done', async () => {
    const t = setup();
    await t.init();
    await t.send(open(1, textOf(1000, { 3: '/* open', 5: 'close */' })));
    t.grace();
    expect(t.drain()).toBeGreaterThan(3);
    const batches = t.tokens();
    expect(batches.length).toBeGreaterThan(3);
    let next = 0;
    for (const b of batches) {
      expect(b).toMatchObject({ doc: 1, exact: true, from: next });
      next += b.ends.length;
    }
    expect(next).toBe(1000);
    expect(t.out.at(-1)).toEqual({ kind: 'done', doc: 1, epoch: 0, version: 1 });
    const first = batches[0];
    // (end offset, metadata): the line's length, the language bits in place of the grammar's.
    expect([...first.data.slice(0, 2)]).toEqual(['line 0'.length, (colour(3) & ~0xff | 9) >>> 0]);
    expect([3, 4, 5, 6].map((l) => t.shown().get(l)![0])).toEqual([5, 5, 5, 3]);
    // Idle: nothing scheduled.
    expect(t.tasks).toHaveLength(0);
  });

  it('answers the viewport first, with a guess from a less indented line above, then the exact pass replaces it', async () => {
    const t = setup();
    await t.init();
    await t.send(open(1, textOf(5000, { 3990: 'block {', 3995: '  /* deep', 4001: '  inside' })));
    await t.send({ kind: 'view', doc: 1, from: 4000, to: 4040 });
    t.tasks.shift()!();
    const [view] = t.tokens();
    expect(view).toMatchObject({ from: 4000, exact: false });
    expect(view.ends).toHaveLength(41);
    // The guess started at line 3990 (indented less than 4000's two spaces), not at the top.
    expect(t.lineCount()).toBeLessThan(200);
    t.drain();
    const exact = t.tokens().filter((m) => m.exact);
    expect(exact[0].from).toBe(0);
    expect(t.shown().get(4001)).toEqual([5, true]);
  });

  it("waits for a new text's first viewport before its pass, so a cold grammar's first work is the lines in view", async () => {
    const t = setup();
    await t.init();
    await t.send(open(1, textOf(3000)));
    expect(t.tasks).toHaveLength(0);
    await t.send({ kind: 'view', doc: 1, from: 2000, to: 2030 });
    t.tasks.shift()!();
    expect(t.tokens()[0]).toMatchObject({ from: 2000, exact: false });
    // Never shown: the pass starts once the grace runs out.
    await t.send(open(2, textOf(300)));
    t.drain();
    expect(t.tokens().some((m) => m.doc === 2)).toBe(false);
    t.grace();
    t.drain();
    expect(t.out.at(-1)).toMatchObject({ kind: 'done', doc: 2 });
  });

  it('reaches a viewport just below the exact pass by running the pass there, exactly', async () => {
    const t = setup();
    await t.init();
    await t.send(open(1, textOf(3000)));
    await t.send({ kind: 'view', doc: 1, from: 40, to: 80 });
    t.tasks.shift()!();
    expect(t.tokens().every((m) => m.exact)).toBe(true);
    expect(t.tokens()[0].from).toBe(0);
  });

  it('stops for a closed text, and drops a cancelled code block', async () => {
    const t = setup();
    await t.init();
    await t.send(open(1, textOf(5000)));
    t.grace();
    t.tasks.shift()!();
    const sent = t.tokens().length;
    await t.send({ kind: 'close', doc: 1 });
    await t.send({ kind: 'code', id: 7, code: 'a', lang: 'toy', theme: 't' });
    await t.send({ kind: 'cancel', id: 7 });
    t.drain();
    expect(t.tokens()).toHaveLength(sent);
    expect(t.out.filter((m) => m.kind === 'code' || m.kind === 'done')).toEqual([]);
  });

  it('answers code blocks before the passes; one in a language Shiki lacks at once, as plain', async () => {
    const t = setup();
    await t.init();
    await t.send(open(1, textOf(5000)));
    t.grace();
    await t.send({ kind: 'code', id: 3, code: 'x = 1', lang: 'toy', theme: 't' });
    await t.send({ kind: 'code', id: 4, code: 'y', lang: 'klingon', theme: 't' });
    expect(t.out).toEqual([{ kind: 'code', id: 4, tokens: null }]);
    t.tasks.shift()!();
    expect(t.out[1]).toEqual({ kind: 'code', id: 3, tokens: { lines: [[{ content: 'x = 1' }]] } });
  });

  it('after an edit, tokenizes from the changed line until the state is the same as before, and no further', async () => {
    const t = setup();
    await t.init();
    await t.send(open(1, textOf(1000)));
    t.grace();
    t.drain();
    const resent = async (c: Extract<ToWorker, { kind: 'edit' }>['changes'], version: number) => {
      t.clear();
      await t.send({ kind: 'edit', doc: 1, version, changes: c });
      t.drain();
      const lines = t.tokens().flatMap((m) => Array.from(m.ends, (_, i) => m.from + i));
      expect(t.tokens().every((m) => m.version === version)).toBe(true);
      return lines.length ? [lines[0], lines.at(-1)] : [];
    };
    // A change that leaves the line's end state alone: that line only.
    expect(await resent([{ startLine: 50, startCol: 0, endLine: 50, endCol: 0, text: 'x' }], 2)).toEqual([50, 50]);
    // Opening a comment changes every line after it.
    expect(await resent([{ startLine: 50, startCol: 0, endLine: 50, endCol: 0, text: '/*' }], 3)).toEqual([50, 999]);
    // Closing it ten lines down: from there to the end again.
    expect(await resent([{ startLine: 60, startCol: 0, endLine: 60, endCol: 0, text: '*/' }], 4)).toEqual([60, 999]);
    expect([55, 60, 61].map((l) => t.shown().get(l)?.[0])).toEqual([undefined, 5, 3]);
    // New lines inside the comment: just them, and the line after them, whose state is kept.
    expect(await resent([{ startLine: 55, startCol: 0, endLine: 55, endCol: 0, text: 'a\nb\n' }], 5)).toEqual([55, 57]);
    expect(t.out.at(-1)).toMatchObject({ kind: 'done', version: 5 });
    // Joining two lines (deleting a line break) outside the comment.
    expect(await resent([{ startLine: 200, startCol: 8, endLine: 201, endCol: 0, text: '' }], 6)).toEqual([200, 200]);
  });

  it('a text the main thread already has (cached) isn\'t tokenized until an edit, then sends from the edit to the end', async () => {
    const t = setup();
    await t.init();
    await t.send(open(1, textOf(300), true));
    await t.send({ kind: 'view', doc: 1, from: 100, to: 140 });
    t.drain();
    expect(t.out).toEqual([]);
    await t.send({ kind: 'edit', doc: 1, version: 2, changes: [{ startLine: 120, startCol: 0, endLine: 120, endCol: 0, text: 'x' }] });
    t.drain();
    const sent = t.tokens().flatMap((m) => Array.from(m.ends, (_, i) => m.from + i));
    expect([sent[0], sent.at(-1), sent.length]).toEqual([120, 299, 180]);
  });

  it('a theme change tokenizes the whole text again, as a new epoch', async () => {
    const t = setup();
    await t.init();
    await t.send(open(1, textOf(100)));
    t.grace();
    t.drain();
    t.clear();
    await t.send({ kind: 'theme', doc: 1, epoch: 1, theme: 'u', known: false });
    t.grace();
    t.drain();
    expect(t.tokens().reduce((n, m) => n + m.ends.length, 0)).toBe(100);
    expect(t.out.every((m) => 'epoch' in m && m.epoch === 1)).toBe(true);
  });
});
