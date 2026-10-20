// @vitest-environment happy-dom
// The Monaco internals the highlighter relies on (monacoInternals.ts), against the installed Monaco.
// If this fails after a Monaco upgrade, the background tokenizer hook moved or changed shape: every
// highlighted file would show plain until monacoInternals.ts follows it.
import { describe, expect, it, vi } from 'vitest';
import * as monaco from 'monaco-editor/editor/editor.api';
import { type BackgroundTokenStore, type TokenizerState, encodedResult, registerTokenization, restartTokenization, tokenRun } from './monacoInternals';

class State implements TokenizerState {
  readonly line: number;
  constructor(line: number) { this.line = line; }
  clone() { return this; }
  equals(o: unknown) { return o === this; }
}

/** What Monaco's token store holds for a line, as (end offset, metadata) pairs. */
function lineTokens(model: monaco.editor.ITextModel, line: number): number[] {
  const t = (model as unknown as { tokenization: { getLineTokens(n: number): { getCount(): number; getEndOffset(i: number): number; getMetadata(i: number): number } } }).tokenization.getLineTokens(line);
  return Array.from({ length: t.getCount() }, (_, i) => [t.getEndOffset(i), t.getMetadata(i)]).flat();
}

describe("Monaco's background tokenizer hook", () => {
  it('creates a background tokenizer per model, takes its tokens and states, and asks the sync side only for edited lines', async () => {
    monaco.languages.register({ id: 'gv-probe' });
    const states = [new State(0), new State(1), new State(2), new State(3)];
    const stores: BackgroundTokenStore[] = [];
    const versions: unknown[] = [];
    const sync = vi.fn((_line: string, _eol: boolean, state: State) => encodedResult(Uint32Array.of(0, 0x8001 * 3), states[state.line + 1]));
    const reg = registerTokenization('gv-probe', {
      getInitialState: () => states[0],
      tokenize: (_line, _eol, state) => ({ tokens: [], endState: state }),
      tokenizeEncoded: sync,
      createBackgroundTokenizer(m, store) {
        // A new model calls this from its own constructor: no version yet (tokens go in later).
        versions.push(m.getVersionId());
        stores.push(store);
        return { dispose: () => {}, requestTokens: () => {} };
      },
    });
    const model = monaco.editor.createModel('one\ntwo\nthree', 'gv-probe');
    expect(stores).toHaveLength(1);
    expect(versions).toEqual([undefined]);
    const store = stores[0];
    expect(typeof store.setTokens).toBe('function');
    expect(typeof store.setEndState).toBe('function');
    expect(typeof store.backgroundTokenizationFinished).toBe('function');

    const meta = (fg: number) => ((fg << 15) | (2 << 24)) >>> 0;
    store.setTokens([tokenRun(2, [Uint32Array.of(1, meta(5), 3, meta(6)), Uint32Array.of(5, meta(7))])]);
    expect(lineTokens(model, 2)).toEqual([1, meta(5), 3, meta(6)]);
    expect(lineTokens(model, 3)).toEqual([5, meta(7)]);

    // With every end state given, Monaco tokenizes nothing itself...
    for (let n = 1; n <= 3; n++) store.setEndState(n, states[n]);
    store.backgroundTokenizationFinished();
    (model as unknown as { tokenization: { forceTokenization(n: number): void } }).tokenization.forceTokenization(3);
    expect(sync).not.toHaveBeenCalled();
    // ...until an edit invalidates a line: then from the state before it, and it stops where the
    // new end state equals the old.
    model.applyEdits([{ range: new monaco.Range(2, 1, 2, 1), text: 'x' }]);
    (model as unknown as { tokenization: { forceTokenization(n: number): void } }).tokenization.forceTokenization(3);
    expect(sync.mock.calls.map((c) => [c[0], c[2]])).toEqual([['xtwo', states[1]]]);

    reg.dispose();
    model.dispose();
  });

  // What its diff editor reads to redraw Inline's removed lines (diffEditorViewZones.js): the
  // original model's state, on that model's token changes only.
  it('reports the tokenization finished on the next token change, not at once; only a write that changes a line is one; a restart puts it back in progress', () => {
    monaco.languages.register({ id: 'gv-probe-state' });
    const state = new State(0);
    const stores: BackgroundTokenStore[] = [];
    const reg = registerTokenization('gv-probe-state', {
      getInitialState: () => state,
      tokenize: (_line, _eol, s) => ({ tokens: [], endState: s }),
      tokenizeEncoded: (_line, _eol, s) => encodedResult(Uint32Array.of(0, 0x8001 * 3), s),
      createBackgroundTokenizer(_m, store) { stores.push(store); return { dispose: () => {}, requestTokens: () => {} }; },
    });
    const model = monaco.editor.createModel('one\ntwo', 'gv-probe-state');
    const internal = model as unknown as { tokenization: { backgroundTokenizationState: number }; onDidChangeTokens(f: () => void): void };
    const completed = () => internal.tokenization.backgroundTokenizationState === 2;
    const seen: boolean[] = [];
    internal.onDidChangeTokens(() => seen.push(completed()));
    const store = stores[0];
    const meta = (fg: number) => ((fg << 15) | (2 << 24)) >>> 0;
    store.setTokens([tokenRun(1, [Uint32Array.of(3, meta(5))])]);
    store.backgroundTokenizationFinished();
    expect(completed()).toBe(true);
    expect(seen).toEqual([false]);
    store.setTokens([tokenRun(1, [Uint32Array.of(3, meta(5))])]);
    expect(seen).toEqual([false]);
    store.setTokens([tokenRun(1, [Uint32Array.of(0, meta(5), 3, meta(5))])]);
    expect(seen).toEqual([false, true]);

    restartTokenization(model);
    expect(completed()).toBe(false);
    store.backgroundTokenizationFinished();
    expect(completed()).toBe(true);
    reg.dispose();
    model.dispose();
  });
});
