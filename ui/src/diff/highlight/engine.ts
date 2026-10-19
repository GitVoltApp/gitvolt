// The highlight worker's engine (highlight.worker.ts): Shiki's TextMate grammars tokenize each open
// text from the top at full speed, off the UI thread, and the lines go back in batches as Monaco's
// encoded tokens. Plain logic over a `Grammars` (shikiGrammars.ts in the app, a fake in tests).
import type { ThemeRegistration } from 'shiki/core';
import {
  type Change,
  type CodeTokens,
  type FromWorker,
  type ThemeInfo,
  type ToWorker,
  BACKGROUND_MASK,
  BACKGROUND_OFFSET,
  FOREGROUND_MASK,
  FOREGROUND_OFFSET,
  LANGUAGE_MASK,
  plainMetadata,
} from './protocol';

/** vscode-textmate's rule stack: the state at a line's end. */
export interface RuleStack { equals(other: RuleStack): boolean }
export interface Grammar { tokenizeLine2(line: string, state: RuleStack | null, timeLimit?: number): { tokens: Uint32Array; ruleStack: RuleStack } }

export interface Grammars {
  init(themes: ThemeRegistration[]): Promise<ThemeInfo[]>;
  /** Shiki ships it (in GitVolt's registry, shikiLanguages.ts). */
  has(lang: string): boolean;
  loaded(lang: string): boolean;
  /** Loads a grammar (and the ones it embeds); resolves to every loaded language id. */
  load(lang: string): Promise<string[]>;
  grammar(lang: string): Grammar;
  /** Selects the theme the grammars colour with; its colour remap (colors.ts), `null` when
   * Shiki's colour ids are Monaco's. */
  theme(name: string): Uint16Array | null;
  code(code: string, lang: string, theme: string): CodeTokens;
}

export type Post = (msg: FromWorker, transfer?: Transferable[]) => void;

/** How long one task tokenizes before it yields to new messages (a viewport, an edit). */
const SLICE_MS = 8;
/** A viewport this close below the exact pass is reached by the pass itself, not guessed. */
const NEAR_LINES = 100;
/** The most lines of one viewport answered at once. */
const VIEW_MAX = 400;
/** How far up a guess looks for the line its construct started on (Monaco's heuristic). */
const GUESS_LOOKBACK = 2000;
/** Longer lines aren't tokenized (one plain token), as @shikijs/monaco did. */
const MAX_LINE = 20_000;
const TIME_LIMIT_MS = 500;
/** How long a new text's pass waits for its first viewport (see `Doc.unseen`). */
const OPEN_GRACE_MS = 200;

const EOL = /\r\n|\r|\n/;

interface Doc {
  id: number;
  epoch: number;
  version: number;
  lang: string;
  langBits: number;
  theme: string;
  lines: string[];
  /** Each line's end state; exact below `frontier`, and in [frontier, validTo) from before an
   * edit (the pass stops once a new state equals one of those). */
  states: Array<RuleStack | null>;
  frontier: number;
  validTo: number;
  /** After an edit, the pass goes at least this far before it may stop; -1: no edit pending. */
  dirtyEnd: number;
  /** Lines below this aren't sent: the main thread has them (a cached text). */
  silent: number;
  /** A cached text: no pass until an edit needs the states. */
  lazy: boolean;
  /** Not on screen yet: the pass waits for the first viewport (or `OPEN_GRACE_MS`), so a cold
   * grammar's first rule compiles go to the lines in view, not to the top of the file. */
  unseen: boolean;
  view: { from: number; to: number } | null;
  touched: number;
  doneSent: boolean;
}

interface Job { id: number; code: string; lang: string; theme: string }

/** Tokens being gathered in one slice: runs of consecutive lines of one doc, posted at its end. */
interface Run { doc: Doc; from: number; exact: boolean; lines: Uint32Array[] }

const firstNonWhitespace = (s: string) => s.search(/\S/);

export class Engine {
  private docs = new Map<number, Doc>();
  private jobs: Job[] = [];
  private scheduled = false;
  private clock = 0;
  private started: Promise<void> | null = null;
  private loading = new Set<string>();
  private readonly g: Grammars;
  private readonly post: Post;
  private readonly now: () => number;
  private readonly yieldTo: (fn: () => void) => void;
  private readonly later: (fn: () => void, ms: number) => void;

  constructor(g: Grammars, post: Post, opts: { now?: () => number; yieldTo?: (fn: () => void) => void; later?: (fn: () => void, ms: number) => void } = {}) {
    this.g = g;
    this.post = post;
    this.now = opts.now ?? (() => performance.now());
    this.yieldTo = opts.yieldTo ?? yieldTask;
    this.later = opts.later ?? ((fn, ms) => setTimeout(fn, ms));
  }

  /** Every message waits for `init` (the highlighter); after that, in order, as they come. */
  handle(msg: ToWorker): void {
    if (msg.kind === 'init') {
      this.started = this.g.init(msg.themes).then(
        (themes) => this.post({ kind: 'ready', themes }),
        (e: unknown) => { this.post({ kind: 'failed', message: String(e) }); throw e; },
      );
      return;
    }
    if (!this.started) throw new Error(`highlight engine: ${msg.kind} before init`);
    void this.started.then(() => this.apply(msg), () => {});
  }

  private apply(msg: Exclude<ToWorker, { kind: 'init' }>): void {
    switch (msg.kind) {
      case 'lang':
        this.g.load(msg.lang).then(
          (loaded) => this.post({ kind: 'lang', id: msg.id, loaded }),
          () => this.post({ kind: 'lang', id: msg.id, loaded: null }),
        );
        return;
      case 'open': {
        const lines = msg.text.split(EOL);
        const d: Doc = {
          id: msg.doc, epoch: msg.epoch, version: msg.version, lang: msg.lang, langBits: msg.langBits, theme: msg.theme,
          lines, states: new Array<RuleStack | null>(lines.length).fill(null), frontier: 0, validTo: 0, dirtyEnd: -1,
          silent: msg.known ? lines.length : 0, lazy: msg.known, unseen: false, view: null, touched: ++this.clock, doneSent: false,
        };
        this.docs.set(msg.doc, d);
        this.awaitView(d);
        break;
      }
      case 'edit': {
        const d = this.docs.get(msg.doc);
        if (!d) return;
        for (const c of msg.changes) applyChange(d, c);
        d.version = msg.version;
        d.unseen = false;
        d.touched = ++this.clock;
        break;
      }
      case 'view': {
        const d = this.docs.get(msg.doc);
        if (!d) return;
        d.view = { from: msg.from, to: msg.to };
        d.unseen = false;
        d.touched = ++this.clock;
        break;
      }
      case 'theme': {
        const d = this.docs.get(msg.doc);
        if (!d) return;
        Object.assign(d, { epoch: msg.epoch, theme: msg.theme, unseen: false, frontier: 0, validTo: 0, dirtyEnd: -1, silent: msg.known ? d.lines.length : 0, lazy: msg.known, doneSent: false });
        d.states.fill(null);
        this.awaitView(d);
        break;
      }
      case 'close':
        this.docs.delete(msg.doc);
        return;
      case 'code':
        if (!this.g.has(msg.lang)) { this.post({ kind: 'code', id: msg.id, tokens: null }); return; }
        this.jobs.push({ id: msg.id, code: msg.code, lang: msg.lang, theme: msg.theme });
        break;
      case 'cancel':
        this.jobs = this.jobs.filter((j) => j.id !== msg.id);
        return;
    }
    this.schedule();
  }

  /** Whether `lang`'s grammar is ready; starts loading it when not. */
  private ready(lang: string): boolean {
    if (this.g.loaded(lang)) return true;
    if (!this.loading.has(lang)) {
      this.loading.add(lang);
      const done = () => { this.loading.delete(lang); this.schedule(); };
      this.g.load(lang).then(done, () => {
        // A grammar that won't load: its blocks are plain, its texts stay as they are.
        this.jobs = this.jobs.filter((j) => j.lang !== lang || (this.post({ kind: 'code', id: j.id, tokens: null }), false));
        for (const [id, d] of this.docs) if (d.lang === lang) this.docs.delete(id);
        done();
      });
    }
    return false;
  }

  private awaitView(d: Doc): void {
    if (d.lazy) return;
    d.unseen = true;
    const epoch = d.epoch;
    this.later(() => {
      if (this.docs.get(d.id) !== d || d.epoch !== epoch || !d.unseen) return;
      d.unseen = false;
      this.schedule();
    }, OPEN_GRACE_MS);
  }

  private needsPass = (d: Doc) => !d.lazy && !d.unseen && d.frontier < d.lines.length;

  /** Whether anything can run now (a grammar still loading starts loading, and schedules when in). */
  private hasWork(): boolean {
    return this.jobs.some((j) => this.ready(j.lang)) || [...this.docs.values()].some((d) => (d.view !== null || this.needsPass(d)) && this.ready(d.lang));
  }

  private schedule(): void {
    if (this.scheduled || !this.hasWork()) return;
    this.scheduled = true;
    this.yieldTo(this.run);
  }

  private run = (): void => {
    this.scheduled = false;
    const deadline = this.now() + SLICE_MS;
    const runs: Run[] = [];
    const done: Doc[] = [];
    while (this.now() < deadline && this.step(runs, done, deadline));
    this.flush(runs, done);
    this.schedule();
  };

  /** One unit of the most urgent work: a code block, a viewport, an edit's re-tokenizing, then
   * the full passes (the most recently opened or viewed text first). False: nothing to do. */
  private step(runs: Run[], done: Doc[], deadline: number): boolean {
    for (let i = 0; i < this.jobs.length; i++) {
      const j = this.jobs[i];
      if (!this.ready(j.lang)) continue;
      this.jobs.splice(i, 1);
      let tokens: CodeTokens | null = null;
      try {
        tokens = this.g.code(j.code, j.lang, j.theme);
      } catch {
        // Plain, as a block in a language Shiki doesn't ship.
      }
      this.post({ kind: 'code', id: j.id, tokens });
      return true;
    }
    const docs = [...this.docs.values()].filter((d) => this.ready(d.lang)).sort((a, b) => b.touched - a.touched);
    for (const d of docs) if (d.view && this.viewStep(d, runs, done, deadline)) return true;
    for (const d of docs) if (d.dirtyEnd >= 0 && this.needsPass(d)) { this.passStep(d, runs, done, deadline); return true; }
    for (const d of docs) if (this.needsPass(d)) { this.passStep(d, runs, done, deadline); return true; }
    return false;
  }

  private viewStep(d: Doc, runs: Run[], done: Doc[], deadline: number): boolean {
    const v = d.view!;
    const from = Math.max(0, v.from);
    const to = Math.min(d.lines.length - 1, v.to, from + VIEW_MAX);
    if (d.lazy || to < d.frontier || from > to) { d.view = null; return false; }
    if (from <= d.frontier + NEAR_LINES) {
      this.passStep(d, runs, done, deadline);
      if (d.frontier > to) d.view = null;
      return true;
    }
    // Monaco's own guess for a viewport its tokenizer hasn't reached: from the nearest line above
    // that's indented less (a construct's start, likely), with the state there if the pass has it,
    // else the initial one. Exact tokens replace it when the pass gets here.
    const remap = this.g.theme(d.theme);
    const before: number[] = [];
    let state: RuleStack | null = null;
    let indent = firstNonWhitespace(d.lines[from]);
    for (let i = from - 1; indent > 0 && i >= 0 && from - i <= GUESS_LOOKBACK; i--) {
      const at = firstNonWhitespace(d.lines[i]);
      if (at < 0 || at >= indent) continue;
      before.push(i);
      indent = at;
      if (i <= d.frontier) { state = i > 0 ? d.states[i - 1] : null; break; }
    }
    for (const i of before.reverse()) state = this.tokenize(d, remap, d.lines[i], state).state;
    for (let i = from; i <= to; i++) {
      const r = this.tokenize(d, remap, d.lines[i], state);
      add(runs, d, i, r.tokens, false);
      state = r.state;
    }
    d.view = null;
    return true;
  }

  /** The exact pass, line by line until the slice's deadline: a cold grammar's first lines (its
   * rules compile as they're first used) can take a while each, and a new viewport waits for them. */
  private passStep(d: Doc, runs: Run[], done: Doc[], deadline: number): void {
    const remap = this.g.theme(d.theme);
    let state = d.frontier > 0 ? d.states[d.frontier - 1] : null;
    for (let i = d.frontier; i < d.lines.length && this.now() < deadline; i++) {
      const r = this.tokenize(d, remap, d.lines[i], state);
      const old = d.states[i];
      d.states[i] = state = r.state;
      if (i >= d.silent) add(runs, d, i, r.tokens, true);
      d.frontier = i + 1;
      // Converged: the lines after kept their tokens and states through the edit.
      if (d.dirtyEnd >= 0 && i >= d.dirtyEnd && i < d.validTo && old !== null && state !== null && old.equals(state)) {
        d.frontier = d.validTo;
        d.dirtyEnd = -1;
        break;
      }
    }
    d.validTo = Math.max(d.validTo, d.frontier);
    if (d.frontier >= d.lines.length) {
      d.dirtyEnd = -1;
      if (!d.doneSent && d.silent === 0) done.push(d);
      d.doneSent = true;
    }
  }

  private tokenize(d: Doc, remap: Uint16Array | null, line: string, state: RuleStack | null): { tokens: Uint32Array; state: RuleStack | null } {
    if (line.length > MAX_LINE) return { tokens: Uint32Array.of(line.length, plainMetadata(d.langBits)), state };
    const r = this.g.grammar(d.lang).tokenizeLine2(line, state, TIME_LIMIT_MS);
    const t = r.tokens;
    const out = new Uint32Array(t.length);
    const keep = ~(LANGUAGE_MASK | FOREGROUND_MASK | BACKGROUND_MASK);
    for (let j = 0; j < t.length; j += 2) {
      out[j] = j + 2 < t.length ? t[j + 2] : line.length;
      const m = t[j + 1];
      let fg = (m & FOREGROUND_MASK) >>> FOREGROUND_OFFSET;
      let bg = (m & BACKGROUND_MASK) >>> BACKGROUND_OFFSET;
      if (remap) { fg = remap[fg] ?? 1; bg = remap[bg] ?? 2; }
      out[j + 1] = ((m & keep) | d.langBits | (fg << FOREGROUND_OFFSET) | (bg << BACKGROUND_OFFSET)) >>> 0;
    }
    return { tokens: out, state: r.ruleStack };
  }

  private flush(runs: Run[], done: Doc[]): void {
    for (const r of runs) {
      if (this.docs.get(r.doc.id) !== r.doc) continue;
      const ends = new Uint32Array(r.lines.length);
      let n = 0;
      r.lines.forEach((l, i) => { n += l.length; ends[i] = n; });
      const data = new Uint32Array(n);
      n = 0;
      for (const l of r.lines) { data.set(l, n); n += l.length; }
      this.post({ kind: 'tokens', doc: r.doc.id, epoch: r.doc.epoch, version: r.doc.version, from: r.from, exact: r.exact, ends, data }, [ends.buffer, data.buffer]);
    }
    for (const d of done) if (this.docs.get(d.id) === d) this.post({ kind: 'done', doc: d.id, epoch: d.epoch, version: d.version });
  }
}

function add(runs: Run[], doc: Doc, line: number, tokens: Uint32Array, exact: boolean): void {
  const last = runs.at(-1);
  if (last && last.doc === doc && last.exact === exact && last.from + last.lines.length === line) last.lines.push(tokens);
  else runs.push({ doc, from: line, exact, lines: [tokens] });
}

/** Applies one content change to a doc's lines and states, keeping the states as Monaco's own
 * store does (the replaced range's last end state stays with the new last line), so an edit that
 * doesn't change how the line ends converges on that line. */
function applyChange(d: Doc, c: Change): void {
  const parts = c.text.split(EOL);
  const head = d.lines[c.startLine].slice(0, c.startCol);
  const tail = d.lines[c.endLine].slice(c.endCol);
  const fresh = parts.map((p, i) => (i === 0 ? head : '') + p + (i === parts.length - 1 ? tail : ''));
  const removed = c.endLine - c.startLine + 1;
  const n = fresh.length;
  const delta = n - removed;
  d.lines.splice(c.startLine, removed, ...fresh);
  d.states.splice(c.startLine, removed - 1, ...new Array<RuleStack | null>(n - 1).fill(null));
  const lastNew = c.startLine + n - 1;
  d.validTo = d.validTo > c.endLine ? d.validTo + delta : Math.min(d.validTo, c.startLine);
  d.frontier = Math.min(d.frontier, c.startLine);
  d.dirtyEnd = Math.max(d.dirtyEnd > c.endLine ? d.dirtyEnd + delta : d.dirtyEnd, lastNew);
  d.silent = Math.min(d.silent > c.endLine ? d.silent + delta : d.silent, c.startLine);
  d.lazy = false;
  d.doneSent = false;
}

/** Yields to the event loop (pending messages run first) without a timer's clamping. */
function yieldTask(fn: () => void): void {
  const ch = new MessageChannel();
  ch.port1.onmessage = () => { ch.port1.close(); fn(); };
  ch.port2.postMessage(null);
}
