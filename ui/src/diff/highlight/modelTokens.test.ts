import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Client, DocReply } from './client';
import type { BackgroundTokenStore, TokenizationSupport, TokenizerState } from './monacoInternals';
import type { ToWorker } from './protocol';

// Monaco's internals, recorded: the support each language registers, and the token runs it stores.
const supports = new Map<string, TokenizationSupport<TokenizerState>>();
vi.mock('./monacoInternals', () => ({
  registerTokenization: (lang: string, s: TokenizationSupport<TokenizerState>) => { supports.set(lang, s); return { dispose() {} }; },
  tokenRun: (from: number, lines: Uint32Array[]) => ({ from, lines }),
  restartTokenization: (model: { restart(): void }) => model.restart(),
  encodedResult: (tokens: Uint32Array, endState: TokenizerState) => ({ tokens, endState }),
}));

const { cache, setTokenTheme, useWorkerTokens } = await import('./modelTokens');

type Listener = (e: unknown) => void;
function fakeModel(text: string) {
  let lines = text.split('\n');
  let version = 1;
  const listeners: Listener[] = [];
  return {
    getValue: () => lines.join('\n'),
    getLineCount: () => lines.length,
    getLineContent: (n: number) => lines[n - 1],
    getLineLength: (n: number) => lines[n - 1].length,
    getVersionId: () => version,
    onDidChangeContent: (l: Listener) => { listeners.push(l); return { dispose: () => listeners.splice(listeners.indexOf(l), 1) }; },
    /** One edit as Monaco reports it (1-based range), applied to the text. */
    edit(startLineNumber: number, startColumn: number, endLineNumber: number, endColumn: number, text: string) {
      const head = lines[startLineNumber - 1].slice(0, startColumn - 1);
      const tail = lines[endLineNumber - 1].slice(endColumn - 1);
      lines.splice(startLineNumber - 1, endLineNumber - startLineNumber + 1, ...(head + text + tail).split('\n'));
      lines = [...lines];
      version++;
      const e = { changes: [{ range: { startLineNumber, startColumn, endLineNumber, endColumn }, text }], versionId: version, isFlush: false, isEolChange: false };
      for (const l of [...listeners]) l(e);
    },
  };
}

/** Monaco's store, as far as its diff editor sees it (diffEditorViewZones.js): it draws Inline's
 * removed lines from the store, and again when the tokenization's completeness, read on each token
 * change (a write that changes a line), differs from the last read. `drawn`: each drawing. */
function fakeStore() {
  const runs: Array<{ from: number; lines: Uint32Array[] }> = [];
  const ends = new Map<number, TokenizerState>();
  const held: number[][] = [];
  const drawn: number[][][] = [];
  let complete = false;
  let read = false;
  const store = {
    setTokens: (r: typeof runs) => {
      runs.push(...r);
      let changed = false;
      for (const run of r) {
        run.lines.forEach((l, i) => {
          changed ||= held[run.from + i - 1]?.join() !== [...l].join();
          held[run.from + i - 1] = [...l];
        });
      }
      if (changed && complete !== read) drawn.push(held.map((l) => [...l]));
      if (changed) read = complete;
    },
    setEndState: (n: number, s: TokenizerState) => void ends.set(n, s),
    backgroundTokenizationFinished: vi.fn(() => { complete = true; }),
  };
  return { store: store as unknown as BackgroundTokenStore & { backgroundTokenizationFinished: ReturnType<typeof vi.fn> }, runs, ends, drawn, restart: () => { complete = false; } };
}

const posted: ToWorker[] = [];
const routes = new Map<number, (m: DocReply) => void>();
const client = { themes: [], post: (m: ToWorker) => posted.push(m), loadLanguage: async () => [], code: () => ({ result: Promise.resolve(null), cancel() {} }), route: (doc: number, to?: (m: DocReply) => void) => (to ? routes.set(doc, to) : routes.delete(doc)) } as unknown as Client;
const visible: Array<{ model: unknown; from: number; to: number }> = [];
const monaco = {
  languages: { getEncodedLanguageId: () => 9 },
  editor: {
    getEditors: () => visible.map((v) => ({ getModel: () => v.model, getVisibleRanges: () => [{ startLineNumber: v.from, endLineNumber: v.to }] })),
    onDidCreateEditor: () => {},
  },
};

setTokenTheme('dark');
useWorkerTokens(monaco as never, client, 'toy');
const support = supports.get('toy')!;

const meta = (fg: number) => ((fg << 15) | (2 << 24) | 9) >>> 0;
/** A line as one token per word, coloured by word index (fg 3, 4, …), as (end, metadata). */
const toks = (line: string) => {
  const out: number[] = [];
  let end = 0;
  line.split(/(?<= )/).forEach((w, i) => { end += w.length; out.push(end, meta(3 + i)); });
  return out;
};
function open(text: string) {
  const s = fakeStore();
  const model = Object.assign(fakeModel(text), { restart: s.restart });
  const before = posted.length;
  const tokenizer = support.createBackgroundTokenizer(model as never, s.store);
  const msg = posted.slice(before).find((m) => m.kind === 'open') as Extract<ToWorker, { kind: 'open' }>;
  const reply = (r: Omit<Extract<DocReply, { kind: 'tokens' }>, 'kind' | 'doc' | 'epoch' | 'ends' | 'data'> & { lines: number[][]; epoch?: number }) => {
    const ends = new Uint32Array(r.lines.length);
    let n = 0;
    r.lines.forEach((l, i) => { n += l.length; ends[i] = n; });
    routes.get(msg.doc)!({ kind: 'tokens', doc: msg.doc, epoch: r.epoch ?? 0, version: r.version, from: r.from, exact: r.exact, ends, data: Uint32Array.from(r.lines.flat()) });
  };
  const done = (version = 1, epoch = 0) => routes.get(msg.doc)!({ kind: 'done', doc: msg.doc, epoch, version });
  /** What Monaco's store shows for a line (1-based), the latest run winning. */
  const shown = (n: number) => { for (const r of [...s.runs].reverse()) if (n >= r.from && n < r.from + r.lines.length) return [...r.lines[n - r.from]]; return undefined; };
  return { model, msg, tokenizer, reply, done, shown, ...s };
}
const lastPosted = () => posted.at(-1)!;

beforeEach(() => { visible.length = 0; });

describe("Monaco's side of the highlight worker", () => {
  it("opens a model in the worker and gives Monaco an end state for every line, so it never tokenizes the file itself", () => {
    const t = open('let a = 1\nlet b = 2\nlet c = 3');
    expect(t.msg).toMatchObject({ kind: 'open', version: 1, lang: 'toy', langBits: 9, theme: 'dark', text: 'let a = 1\nlet b = 2\nlet c = 3', known: false });
    expect([...t.ends.keys()]).toEqual([1, 2, 3]);
    expect(t.runs).toEqual([]);
  });

  it("puts the worker's batches in Monaco's store, as copies it may edit; a guess never replaces exact tokens", () => {
    const t = open('let a = 1\nlet b = 2\nlet c = 3');
    t.reply({ version: 1, from: 1, exact: false, lines: [toks('let b = 2'), toks('let c = 3')] });
    t.reply({ version: 1, from: 0, exact: true, lines: [toks('let a = 1'), toks('let b = 2')] });
    t.reply({ version: 1, from: 1, exact: false, lines: [[9, meta(7)]] });
    expect(t.shown(1)).toEqual(toks('let a = 1'));
    expect(t.shown(2)).toEqual(toks('let b = 2'));
    expect(t.shown(3)).toEqual(toks('let c = 3'));
    const copies = t.runs.flatMap((r) => r.lines);
    expect(new Set(copies.map((l) => l.buffer)).size).toBe(copies.length);
  });

  it('keeps a finished text in the cache: the same text (or a theme already seen) opens coloured at once', async () => {
    const text = 'const x = 1\nconst y = 2';
    const t = open(text);
    t.reply({ version: 1, from: 0, exact: true, lines: [toks('const x = 1'), toks('const y = 2')] });
    t.done();
    expect(t.store.backgroundTokenizationFinished).toHaveBeenCalled();
    const again = open(text);
    expect(again.msg.known).toBe(true);
    // Once the model is built (a microtask: Monaco calls this from the model's constructor).
    expect(again.runs).toEqual([]);
    await Promise.resolve();
    expect(again.shown(2)).toEqual(toks('const y = 2'));
    expect(again.store.backgroundTokenizationFinished).toHaveBeenCalled();
    // An unfinished text isn't kept.
    const partial = open('const z = 3\nconst w = 4');
    partial.reply({ version: 1, from: 0, exact: true, lines: [toks('const z = 3')] });
    partial.done();
    expect(open('const z = 3\nconst w = 4').msg.known).toBe(false);
    again.tokenizer.dispose();
    t.tokenizer.dispose();
  });

  it("says the text is finished before a token change Monaco sees, so its diff editor draws Inline's removed lines again: after the worker's pass, and from the cache", async () => {
    const exact = [toks('fn one() {'), [0, meta(1)], toks('  two()')];
    const t = open('fn one() {\n\n  two()');
    // The lines on screen already hold the same tokens, guessed: the store reports no change for them.
    t.reply({ version: 1, from: 0, exact: false, lines: exact });
    t.reply({ version: 1, from: 0, exact: true, lines: exact });
    expect(t.drawn).toEqual([]);
    t.done();
    expect(t.drawn).toEqual([exact]);
    const again = open('fn one() {\n\n  two()');
    await Promise.resolve();
    expect(again.drawn).toEqual([exact]);
    again.tokenizer.dispose();
    t.tokenizer.dispose();
  });

  it("a theme change puts the text back in progress, so Monaco's diff editor draws Inline's removed lines in the new theme's tokens once they're in", () => {
    const t = open('paint me\nagain');
    t.reply({ version: 1, from: 0, exact: true, lines: [[8, meta(3)], [5, meta(4)]] });
    t.done();
    setTokenTheme('light');
    t.reply({ version: 1, from: 0, exact: true, lines: [[8, meta(5)], [5, meta(6)]], epoch: 1 });
    t.done(1, 1);
    expect(t.drawn.at(-1)).toEqual([[8, meta(5)], [5, meta(6)]]);
    // Back to a theme the cache has it in: at once.
    setTokenTheme('dark');
    expect(t.drawn.at(-1)).toEqual([[8, meta(3)], [5, meta(4)]]);
    t.tokenizer.dispose();
  });

  it('asks the worker for the lines on screen while any is unknown, once per range', () => {
    const t = open(Array.from({ length: 100 }, (_, i) => `line ${i}`).join('\n'));
    visible.push({ model: t.model, from: 50, to: 70 });
    const views = () => posted.filter((m) => m.kind === 'view' && m.doc === t.msg.doc);
    (t.tokenizer as unknown as { viewsChanged(): void }).viewsChanged();
    (t.tokenizer as unknown as { viewsChanged(): void }).viewsChanged();
    expect(views()).toEqual([{ kind: 'view', doc: t.msg.doc, from: 49, to: 69 }]);
    t.reply({ version: 1, from: 49, exact: false, lines: Array.from({ length: 21 }, () => [7, meta(3)]) });
    visible[0] = { model: t.model, from: 52, to: 68 };
    (t.tokenizer as unknown as { viewsChanged(): void }).viewsChanged();
    expect(views()).toHaveLength(1);
  });

  it("an edit goes to the worker; meanwhile the edited line keeps its tokens, shifted, and Monaco's sync tokenizer serves them", () => {
    const t = open('let a = 1\nlet bb = 2\nlet c = 3');
    t.reply({ version: 1, from: 0, exact: true, lines: [toks('let a = 1'), toks('let bb = 2'), toks('let c = 3')] });
    t.model.edit(2, 5, 2, 7, 'b');
    expect(lastPosted()).toEqual({ kind: 'edit', doc: t.msg.doc, version: 2, changes: [{ startLine: 1, startCol: 4, endLine: 1, endCol: 6, text: 'b' }] });
    // Monaco re-tokenizes the edited line from the end state of the line before it.
    const r = support.tokenizeEncoded('let b = 2', true, t.ends.get(1)!) as unknown as { tokens: Uint32Array; endState: TokenizerState };
    expect([...r.tokens]).toEqual([0, meta(3), 4, meta(4), 6, meta(5), 8, meta(6)]);
    // Its end state is the line's own, unchanged: Monaco stops there.
    expect(r.endState).toBe(t.ends.get(2));
    // Line 1 starts from the shared initial state: found by its text.
    const first = support.tokenizeEncoded('let a = 1', true, support.getInitialState()) as unknown as { tokens: Uint32Array; endState: TokenizerState };
    expect(first.endState).toBe(t.ends.get(1));
    expect(first.tokens[1]).toBe(meta(3));
  });

  it("maps a batch the worker made before an edit it hadn't seen: lines after move, the edited line waits for its new tokens", () => {
    const t = open('a\nb\nc\nd');
    t.model.edit(2, 2, 2, 2, '\nnew');
    t.reply({ version: 1, from: 0, exact: true, lines: [[1, meta(3)], [1, meta(4)], [1, meta(5)], [1, meta(6)]] });
    expect(t.shown(1)).toEqual([1, meta(3)]);
    expect(t.shown(2)).toBeUndefined();
    expect(t.shown(4)).toEqual([1, meta(5)]);
    expect(t.shown(5)).toEqual([1, meta(6)]);
    // The worker's answer at the new version lands as it is.
    t.reply({ version: 2, from: 1, exact: true, lines: [[1, meta(8)], [3, meta(8)]] });
    expect(t.shown(3)).toEqual([3, meta(8)]);
    expect(t.ends.get(3)).toBeDefined();
  });

  it('follows the app theme: from the cache when the text was seen in that theme, else the worker again, in a new epoch', () => {
    const text = 'theme me\nplease';
    const t = open(text);
    t.reply({ version: 1, from: 0, exact: true, lines: [[8, meta(3)], [6, meta(4)]] });
    t.done();
    setTokenTheme('light');
    expect(lastPosted()).toEqual({ kind: 'theme', doc: t.msg.doc, epoch: 1, theme: 'light', known: false });
    // A batch of the old epoch is dropped.
    t.reply({ version: 1, from: 0, exact: true, lines: [[8, meta(9)]], epoch: 0 });
    expect(t.shown(1)).toEqual([8, meta(3)]);
    t.reply({ version: 1, from: 0, exact: true, lines: [[8, meta(5)], [6, meta(5)]], epoch: 1 });
    t.done(1, 1);
    setTokenTheme('dark');
    expect(lastPosted()).toMatchObject({ kind: 'theme', epoch: 2, theme: 'dark', known: true });
    expect(t.shown(1)).toEqual([8, meta(3)]);
    t.tokenizer.dispose();
    expect(lastPosted()).toEqual({ kind: 'close', doc: t.msg.doc });
    expect(routes.has(t.msg.doc)).toBe(false);
    expect(cache.size).toBeGreaterThan(0);
  });
});
