// Monaco's tokenization internals, kept to this one module. Monaco 0.57's public `setTokensProvider`
// wraps a provider in an adapter that drops `createBackgroundTokenizer`, the hook VS Code's
// TextMate worker feeds its tokens through (TokenizerSyntaxTokenBackend): so the highlighter puts
// its support in Monaco's TokenizationRegistry itself, and hands the background store Monaco's own
// token runs. monacoInternals.test.ts checks all of this against the installed Monaco: an upgrade
// that moves or reshapes any of it fails there, loudly, instead of leaving every file plain.
import { EncodedTokenizationResult, TokenizationRegistry } from 'monaco-editor/editor/common/languages';
import { ContiguousMultilineTokens } from 'monaco-editor/editor/common/tokens/contiguousMultilineTokens';
import type { Monaco } from '../monaco/setup';

export type Model = ReturnType<Monaco['editor']['createModel']>;

export interface TokenizerState { clone(): TokenizerState; equals(other: unknown): boolean }

/** Monaco's IBackgroundTokenizationStore: where a background tokenizer puts what it found. */
export interface BackgroundTokenStore {
  setTokens(tokens: ContiguousMultilineTokens[]): void;
  /** Accepted only for a line Monaco doesn't already hold a valid state for. */
  setEndState(lineNumber: number, state: TokenizerState): void;
  backgroundTokenizationFinished(): void;
}

/** Monaco's IBackgroundTokenizer. `requestTokens`: Monaco overwrote lines with its own guess. */
export interface BackgroundTokenizer { dispose(): void; requestTokens(startLineNumber: number, endLineNumberExclusive: number): void }

/** Monaco's ITokenizationSupport, for a language tokenized in the background. */
export interface TokenizationSupport<S extends TokenizerState> {
  getInitialState(): S;
  tokenize(line: string, hasEOL: boolean, state: S): { tokens: unknown[]; endState: S };
  tokenizeEncoded(line: string, hasEOL: boolean, state: S): EncodedTokenizationResult;
  createBackgroundTokenizer(model: Model, store: BackgroundTokenStore): BackgroundTokenizer;
}

export const registerTokenization = <S extends TokenizerState>(languageId: string, support: TokenizationSupport<S>): { dispose(): void } =>
  TokenizationRegistry.register(languageId, support);

/** `tokenizeEncoded`'s result: one line's tokens as (start offset, metadata) pairs. */
export const encodedResult = (tokens: Uint32Array, endState: TokenizerState): EncodedTokenizationResult => new EncodedTokenizationResult(tokens, [], endState);

/** Lines `from`… (1-based) for `BackgroundTokenStore.setTokens`, as (end offset, metadata) pairs.
 * Monaco keeps the arrays and edits them in place as the text changes: pass ones it can own. */
export const tokenRun = (from: number, lines: Uint32Array[]): ContiguousMultilineTokens => new ContiguousMultilineTokens(from, lines);
