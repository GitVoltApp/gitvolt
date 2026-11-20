// Types for the Monaco internals diffWork.ts uses (monaco-editor ships none for them).
declare module 'monaco-editor/editor/standalone/browser/standaloneServices' {
  export const StandaloneServices: { get(id: object): unknown };
}

declare module 'monaco-editor/editor/browser/widget/diffEditor/diffProviderFactoryService' {
  export const IDiffProviderFactoryService: object;
}

declare module 'monaco-editor/editor/common/services/editorWorker' {
  export const IEditorWorkerService: object;
}
