/**
 * The seam between a repo view (the startup chunk) and the shared editor (the lazy Monaco chunk)
 * for what the view's own code can't reach. Kept free of imports, so the startup chunk stays
 * small; both are no-ops until the editor has loaded.
 *
 * One case its attach cleanups can't cover (J16): a kept diff panel that unmounts while hidden.
 * Hiding ran the cleanup, which kept the editor because its box was still in the document; React
 * runs no cleanup again on the later unmount. The view calls `releaseDetachedEditors` when it
 * unmounts, and the host lets go of any box that has left the document (`releaseDetached`).
 *
 * And the store's swap of a WIP's open file for the same file in the commit that took the WIP
 * row's place (`keepEditorPlace`): the reader's place stays where it was.
 */
let release: (() => void) | null = null;
let keepPlace: (() => void) | null = null;

/** Set by the editor's loader (`useMonacoHost`) once the host exists. */
export function setEditorRelease(fn: () => void, keep: (() => void) | null = null): void {
  release = fn;
  keepPlace = keep;
}

export function releaseDetachedEditors(): void {
  release?.();
}

/** The next show of the same path, in Diff or File View, keeps the editor's scroll and cursor
 * (the host's `keepViewOnNextShow`). */
export function keepEditorPlace(): void {
  keepPlace?.();
}
