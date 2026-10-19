// The highlight worker's messages (highlight.worker.ts, engine.ts) and the token encoding they
// carry. Lines and columns are 0-based throughout; Monaco's 1-based ones convert at the edge
// (modelTokens.ts).
import type { ThemeRegistration, ThemeRegistrationResolved } from 'shiki/core';

/** A Markdown code block's tokens, coloured (ui/src/markdown/MdCode.tsx). */
export interface CodeTokens { lines: Array<Array<{ content: string; color?: string; fontStyle?: number }>>; fg?: string; bg?: string }

/** One editor theme as the worker loaded it: Shiki's resolved theme (Monaco's theme is made from
 * it) and the token colours to give Monaco as `encodedTokensColors`, in the order the worker's
 * token metadata numbers them (colors.ts). */
export interface ThemeInfo { name: string; theme: ThemeRegistrationResolved; colors: string[] }

/** One of Monaco's content changes: the old range, and the text that replaced it. */
export interface Change { startLine: number; startCol: number; endLine: number; endCol: number; text: string }

export type ToWorker =
  | { kind: 'init'; themes: ThemeRegistration[] }
  | { kind: 'lang'; id: number; lang: string }
  /** `known`: the main thread already has this text's tokens (the cache): no pass until an edit
   * needs the grammar's states. */
  | { kind: 'open'; doc: number; epoch: number; version: number; lang: string; langBits: number; theme: string; text: string; known: boolean }
  | { kind: 'edit'; doc: number; version: number; changes: Change[] }
  /** The lines on screen (inclusive): answered before any full pass goes on. */
  | { kind: 'view'; doc: number; from: number; to: number }
  /** Tokenize again from the top in another theme (`known` as for `open`). */
  | { kind: 'theme'; doc: number; epoch: number; theme: string; known: boolean }
  | { kind: 'close'; doc: number }
  | { kind: 'code'; id: number; code: string; lang: string; theme: string }
  | { kind: 'cancel'; id: number };

export type FromWorker =
  | { kind: 'ready'; themes: ThemeInfo[] }
  | { kind: 'failed'; message: string }
  /** The languages loaded so far (embedded ones too), or `null` when this one failed to load. */
  | { kind: 'lang'; id: number; loaded: string[] | null }
  /** Lines `from`… in Monaco's encoding: line `from + i` is `data[ends[i - 1] ?? 0 … ends[i])`,
   * pairs of (token end offset, metadata). `exact`: tokenized from the true state of the line
   * before; otherwise a viewport's guess, replaced when the pass gets there. */
  | { kind: 'tokens'; doc: number; epoch: number; version: number; from: number; exact: boolean; ends: Uint32Array; data: Uint32Array }
  /** A full pass reached the end with every line exact, at `version`. */
  | { kind: 'done'; doc: number; epoch: number; version: number }
  | { kind: 'code'; id: number; tokens: CodeTokens | null };

// Token metadata, as vscode-textmate and Monaco both lay it out (Monaco's MetadataConsts).
export const LANGUAGE_MASK = 0x0000_00ff;
export const BALANCED_BRACKETS_MASK = 0x0000_0400;
export const FOREGROUND_MASK = 0x00ff_8000;
export const BACKGROUND_MASK = 0xff00_0000;
export const FOREGROUND_OFFSET = 15;
export const BACKGROUND_OFFSET = 24;

/** A plain token's metadata: the default colours (ids 1 and 2 in every theme's colour map). */
export const plainMetadata = (langBits: number): number => (langBits | (1 << FOREGROUND_OFFSET) | (2 << BACKGROUND_OFFSET)) >>> 0;
