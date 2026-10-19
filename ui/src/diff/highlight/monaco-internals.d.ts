// Types for the Monaco internals monacoInternals.ts uses (monaco-editor ships none for them).
declare module 'monaco-editor/editor/common/languages' {
  export const TokenizationRegistry: {
    register(languageId: string, support: object): { dispose(): void };
    get(languageId: string): object | null;
  };
  export class EncodedTokenizationResult {
    constructor(tokens: Uint32Array, fontInfo: unknown[], endState: object);
    readonly tokens: Uint32Array;
    readonly endState: object;
  }
}

declare module 'monaco-editor/editor/common/tokens/contiguousMultilineTokens' {
  export class ContiguousMultilineTokens {
    constructor(startLineNumber: number, tokens: Uint32Array[]);
    readonly startLineNumber: number;
    readonly endLineNumber: number;
    getLineTokens(lineNumber: number): Uint32Array;
  }
}
