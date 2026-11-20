// Types for the Monaco internals diffWork.ts and deletedZone.ts use (monaco-editor ships none for
// them): only the members they use.
declare module 'monaco-editor/editor/standalone/browser/standaloneServices' {
  export const StandaloneServices: { get(id: object): unknown };
}

declare module 'monaco-editor/editor/browser/widget/diffEditor/diffProviderFactoryService' {
  export const IDiffProviderFactoryService: object;
}

declare module 'monaco-editor/editor/common/services/editorWorker' {
  export const IEditorWorkerService: object;
}

declare module 'monaco-editor/editor/browser/widget/diffEditor/components/diffEditorViewZones/renderLines' {
  export interface LineTokensLike { getLineContent(): string }
  export interface LineBreakDataLike { breakOffsets: number[] }
  export interface LineSourceLike {
    lineTokens: LineTokensLike[];
    lineBreakData: (LineBreakDataLike | null | undefined)[];
    mightContainNonBasicASCII: boolean;
    mightContainRTL: boolean;
  }
  export class LineSource implements LineSourceLike {
    constructor(lineTokens: LineTokensLike[], lineBreakData?: (LineBreakDataLike | null | undefined)[], mightContainNonBasicASCII?: boolean, mightContainRTL?: boolean);
    lineTokens: LineTokensLike[];
    lineBreakData: (LineBreakDataLike | null | undefined)[];
    mightContainNonBasicASCII: boolean;
    mightContainRTL: boolean;
  }
  export interface RenderOptionsLike {
    tabSize: number;
    fontInfo: object;
    typicalHalfwidthCharacterWidth: number;
    scrollBeyondLastColumn: number;
    lineHeight: number;
    lineDecorationsWidth: number;
  }
  export interface PositionLike { lineNumber: number; column: number; delta(deltaLineNumber?: number, deltaColumn?: number): PositionLike }
  export interface RenderLinesResultLike {
    heightInLines: number;
    minWidthInPx: number;
    viewLineCounts: number[];
    getModelPositionAt(domNode: Node, offset: number): PositionLike | undefined;
  }
  export function renderLines(source: LineSourceLike, options: RenderOptionsLike, decorations: unknown[], domNode: HTMLElement, noExtra?: boolean): RenderLinesResultLike;
}

/** `deletedZoneHook` is build/monacoDeletedZones.ts's addition. */
declare module 'monaco-editor/editor/browser/widget/diffEditor/components/diffEditorViewZones/diffEditorViewZones' {
  export const deletedZoneHook: { render?: unknown } | undefined;
}

declare module 'monaco-editor/editor/browser/config/domFontInfo' {
  export function applyFontInfo(domNode: HTMLElement, fontInfo: object): void;
}

declare module 'monaco-editor/editor/common/core/range' {
  export class Range {
    constructor(startLineNumber: number, startColumn: number, endLineNumber: number, endColumn: number);
    readonly startLineNumber: number;
    readonly startColumn: number;
    readonly endLineNumber: number;
    readonly endColumn: number;
    delta(lineCount: number): Range;
  }
}

declare module 'monaco-editor/editor/common/viewModel/inlineDecorations' {
  import type { Range } from 'monaco-editor/editor/common/core/range';
  export class InlineDecoration {
    constructor(range: Range, inlineClassName: string, type: number);
    readonly range: Range;
    readonly inlineClassName: string;
    readonly type: number;
  }
}
