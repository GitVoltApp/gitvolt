// Monaco's side of the highlight worker. Each highlighted language gets a tokenization support
// whose background tokenizer (monacoInternals.ts) is a `ModelTokens` per model: it sends the text to
// the worker, puts the worker's batches in Monaco's token store, and keeps a copy of every line's
// tokens. Monaco is handed an end state for every line up front, so it never tokenizes the file
// itself; after an edit it re-tokenizes the edited lines synchronously, which the support answers
// from that copy (adjusted for the edit) until the worker's exact tokens arrive.
import { textKey } from '../../util/textHash';
import type { Monaco } from '../monaco/setup';
import type { Client, DocReply } from './client';
import {
  type BackgroundTokenStore,
  type BackgroundTokenizer,
  type Model,
  type TokenizationSupport,
  type TokenizerState,
  encodedResult,
  registerTokenization,
  tokenRun,
} from './monacoInternals';
import { type Change, plainMetadata } from './protocol';
import { type Lines, TokenCache } from './tokenCache';
import { editTokens } from './tokenEdits';

type ContentChange = Parameters<Parameters<Model['onDidChangeContent']>[0]>[0];
type Editor = ReturnType<Monaco['editor']['getEditors']>[number];

/** Recently highlighted texts' tokens (tokenCache.ts): an 8,000-line file takes about 1 MB. */
export const cache = new TokenCache(16 * 1024 * 1024, 32);
const cacheKey = (lang: string, theme: string, text: string) => `${lang}\n${theme}\n${textKey(text)}`;

/**
 * A line's end state, as Monaco's store keeps it: a handle that knows its doc and current line
 * (renumbered as lines move), so the sync tokenizer can answer the next line from the doc's copy.
 * Equal only to itself: an edited line that keeps its handle has converged.
 */
class LineState implements TokenizerState {
  doc: ModelTokens | null;
  line: number;
  constructor(doc: ModelTokens | null, line: number) {
    this.doc = doc;
    this.line = line;
  }
  clone() { return this; }
  equals(other: unknown) { return other === this; }
}
const INITIAL = new LineState(null, 0);

/** No tokens known for the line. */
const UNKNOWN = 0;
/** A viewport's guess, or an edited line's adjusted tokens. */
const GUESS = 1;
const EXACT = 2;

let client: Client | null = null;
let monacoRef: Monaco | null = null;
let theme = '';
let nextDoc = 0;
let clock = 0;
const live = new Set<ModelTokens>();
const byModel = new WeakMap<Model, ModelTokens>();

export class ModelTokens implements BackgroundTokenizer {
  readonly id = ++nextDoc;
  /** Each line's tokens ((end offset, metadata) pairs); `undefined` shows plain. */
  private toks: Array<Uint32Array | undefined>;
  private known: number[];
  readonly handles: LineState[];
  private epoch = 0;
  private key: string | null;
  /** The edits the worker may not have seen yet, to map its lines through. */
  private edits: Array<{ version: number; changes: Change[] }> = [];
  private viewSent: { from: number; to: number } | null = null;
  private disposed = false;
  private sub: { dispose(): void };
  lastEdit = 0;
  readonly model: Model;
  readonly lang: string;
  private store: BackgroundTokenStore;
  private langBits: number;

  constructor(model: Model, store: BackgroundTokenStore, lang: string, langBits: number) {
    this.model = model;
    this.store = store;
    this.lang = lang;
    this.langBits = langBits;
    const text = model.getValue();
    const n = model.getLineCount();
    this.key = cacheKey(lang, theme, text);
    const cached = cache.get(this.key);
    this.toks = cached ? [...cached] : new Array<Uint32Array | undefined>(n);
    this.known = new Array<number>(n).fill(cached ? EXACT : UNKNOWN);
    this.handles = Array.from({ length: n }, (_, i) => new LineState(this, i + 1));
    for (let i = 0; i < n; i++) store.setEndState(i + 1, this.handles[i]);
    // A new model makes its tokenizer from its own constructor, before it can take tokens (or has
    // a version: its first is 1). The cached ones go in once it's built, still before any paint.
    if (cached) {
      queueMicrotask(() => {
        if (this.disposed) return;
        this.show(0, n);
        store.backgroundTokenizationFinished();
      });
    }
    live.add(this);
    byModel.set(model, this);
    this.sub = model.onDidChangeContent((e) => this.edited(e));
    client!.route(this.id, (m) => this.receive(m));
    const version = (model.getVersionId() as number | undefined) ?? 1;
    client!.post({ kind: 'open', doc: this.id, epoch: this.epoch, version, lang, langBits, theme, text, known: !!cached });
    queueMicrotask(() => this.viewsChanged());
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.sub.dispose();
    live.delete(this);
    if (byModel.get(this.model) === this) byModel.delete(this.model);
    for (const h of this.handles) h.line = -1;
    client!.route(this.id, undefined);
    client!.post({ kind: 'close', doc: this.id });
  }

  /** Monaco overwrote these lines with a guess of its own: put ours back. */
  requestTokens(from: number, toExclusive: number): void {
    this.show(from - 1, Math.min(toExclusive - 1, this.toks.length));
  }

  /** Lines `[from, to)` (0-based) from the copy into Monaco's store. Monaco edits its arrays in
   * place as the text changes, so it gets copies; plain lines go as plain. */
  private show(from: number, to: number): void {
    if (to <= from) return;
    const plain = plainMetadata(this.langBits);
    const lines: Uint32Array[] = [];
    for (let i = from; i < to; i++) lines.push(this.toks[i]?.slice() ?? Uint32Array.of(this.model.getLineLength(i + 1), plain));
    this.store.setTokens([tokenRun(from + 1, lines)]);
  }

  /** The sync tokenizer's answer for line `line` (1-based): the copy's tokens as start offsets. */
  tokensFor(line: number, text: string): Uint32Array {
    const t = this.toks[line - 1];
    if (!t) return Uint32Array.of(0, plainMetadata(this.langBits));
    const out: number[] = [];
    for (let j = 0, start = 0; j < t.length && start < Math.max(text.length, 1); start = t[j], j += 2) out.push(start, t[j + 1]);
    return Uint32Array.from(out);
  }

  get lineCount(): number { return this.toks.length; }

  private receive(m: DocReply): void {
    if (this.disposed || m.epoch !== this.epoch) return;
    while (this.edits.length > 0 && this.edits[0].version <= m.version) this.edits.shift();
    if (m.kind === 'done') {
      if (m.version !== this.model.getVersionId() || this.known.some((k) => k !== EXACT)) return;
      cache.set(this.key ??= cacheKey(this.lang, theme, this.model.getValue()), [...this.toks] as Lines);
      this.store.backgroundTokenizationFinished();
      return;
    }
    let runFrom = -1;
    let prev = -2;
    const flush = () => { if (runFrom >= 0) this.show(runFrom, prev + 1); runFrom = -1; };
    for (let i = 0; i < m.ends.length; i++) {
      const line = this.mapLine(m.from + i);
      if (line < 0 || line >= this.toks.length || (!m.exact && this.known[line] === EXACT)) continue;
      this.toks[line] = m.data.subarray(i > 0 ? m.ends[i - 1] : 0, m.ends[i]);
      this.known[line] = m.exact ? EXACT : GUESS;
      if (m.exact) this.store.setEndState(line + 1, this.handles[line]);
      if (line !== prev + 1) { flush(); runFrom = line; }
      prev = line;
    }
    flush();
  }

  /** A line the worker numbered before the edits it hadn't seen: its number now, or -1 if one of
   * them changed it (the worker sends it again). */
  private mapLine(line: number): number {
    for (const e of this.edits) {
      for (const c of e.changes) {
        if (line < c.startLine) continue;
        if (line <= c.endLine) return -1;
        line += c.text.split(/\r\n|\r|\n/).length - 1 - (c.endLine - c.startLine);
      }
    }
    return line;
  }

  private edited(e: ContentChange): void {
    if (this.disposed || e.isFlush || e.isEolChange) return;
    const changes: Change[] = e.changes.map((c) => ({ startLine: c.range.startLineNumber - 1, startCol: c.range.startColumn - 1, endLine: c.range.endLineNumber - 1, endCol: c.range.endColumn - 1, text: c.text }));
    let first = Infinity;
    for (const c of changes) {
      const parts = c.text.split(/\r\n|\r|\n/);
      const removed = c.endLine - c.startLine + 1;
      this.toks.splice(c.startLine, removed, ...editTokens(this.toks[c.startLine], this.toks[c.endLine], c.startCol, c.endCol, parts));
      this.known.splice(c.startLine, removed, ...parts.map(() => GUESS));
      // As Monaco's store does: the replaced range's last end state stays with the new last line.
      const gone = this.handles.splice(c.startLine, removed - 1, ...parts.slice(1).map(() => new LineState(this, 0)));
      for (const h of gone) h.line = -1;
      first = Math.min(first, c.startLine);
    }
    for (let i = first; i < this.handles.length; i++) this.handles[i].line = i + 1;
    this.key = null;
    this.lastEdit = ++clock;
    this.edits.push({ version: e.versionId, changes });
    client!.post({ kind: 'edit', doc: this.id, version: e.versionId, changes });
  }

  /** The app theme changed: this text's tokens in the new one, from the cache or the worker. */
  retheme(): void {
    this.epoch++;
    this.edits = [];
    this.key = cacheKey(this.lang, theme, this.model.getValue());
    const cached = cache.get(this.key);
    if (cached && cached.length === this.toks.length) {
      this.toks = [...cached];
      this.known.fill(EXACT);
      this.show(0, this.toks.length);
    } else {
      this.known.fill(UNKNOWN);
    }
    this.viewSent = null;
    client!.post({ kind: 'theme', doc: this.id, epoch: this.epoch, theme, known: !!cached });
    this.viewsChanged();
  }

  /** The lines on screen in the editors showing this model: asks the worker for them first when
   * any isn't known yet. */
  viewsChanged(): void {
    if (this.disposed) return;
    let from = Infinity;
    let to = -1;
    for (const ed of monacoRef?.editor.getEditors() ?? []) {
      if (ed.getModel() !== this.model) continue;
      for (const r of ed.getVisibleRanges()) {
        from = Math.min(from, r.startLineNumber - 1);
        to = Math.max(to, r.endLineNumber - 1);
      }
    }
    if (to < 0) return;
    to = Math.min(to, this.known.length - 1);
    let unknown = false;
    for (let i = from; i <= to && !unknown; i++) unknown = this.known[i] === UNKNOWN;
    if (!unknown || (this.viewSent && this.viewSent.from === from && this.viewSent.to === to)) return;
    this.viewSent = { from, to };
    client!.post({ kind: 'view', doc: this.id, from, to });
  }
}

/** The support Monaco calls for a highlighted language (one per language). */
function support(lang: string, langBits: number): TokenizationSupport<LineState> {
  const plain = Uint32Array.of(0, plainMetadata(langBits));
  return {
    getInitialState: () => INITIAL,
    tokenize: (_line, _eol, state) => ({ tokens: [], endState: state }),
    tokenizeEncoded(line, _eol, state) {
      // Line 1 starts from the shared initial state: its doc is the one whose first line this is
      // (the most recently edited, for texts that start alike).
      let doc = state.doc;
      if (!doc && state === INITIAL) {
        for (const d of live) if (d.lang === lang && d.lineCount > 0 && d.model.getLineContent(1) === line && (!doc || d.lastEdit > doc.lastEdit)) doc = d;
      }
      const n = state.line + 1;
      if (!doc || state.line < 0 || n > doc.lineCount) return encodedResult(plain, state);
      return encodedResult(doc.tokensFor(n, line), doc.handles[n - 1]);
    },
    createBackgroundTokenizer: (model, store) => new ModelTokens(model, store, lang, langBits),
  };
}

const registered = new Set<string>();

/** Highlights `lang`'s models through the worker from now on (once per language). */
export function useWorkerTokens(monaco: Monaco, c: Client, lang: string): void {
  client = c;
  if (!monacoRef) watchEditors((monacoRef = monaco));
  if (registered.has(lang)) return;
  registered.add(lang);
  registerTokenization(lang, support(lang, monaco.languages.getEncodedLanguageId(lang)));
}

/** The editor theme the tokens are coloured for; every open text follows a change. */
export function setTokenTheme(name: string): void {
  if (name === theme) return;
  const first = theme === '';
  theme = name;
  if (!first) for (const d of live) d.retheme();
}

/** Every editor's scrolling and layout: what's on screen goes to the worker first. */
function watchEditors(monaco: Monaco): void {
  const track = (ed: Editor) => {
    const changed = () => { const m = ed.getModel(); if (m) byModel.get(m)?.viewsChanged(); };
    ed.onDidScrollChange(changed);
    ed.onDidLayoutChange(changed);
    ed.onDidChangeModel(changed);
    ed.onDidChangeHiddenAreas(changed);
  };
  monaco.editor.getEditors().forEach(track);
  monaco.editor.onDidCreateEditor(track);
}
