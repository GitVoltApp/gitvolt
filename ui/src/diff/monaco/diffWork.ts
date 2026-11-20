// Monaco's diff computation internals, kept to this one module (as highlight/monacoInternals.ts):
// the algorithm each diff is computed with, and a restart of the editor worker that computes them.
// diffWork.test.ts checks both against the installed Monaco: an upgrade that moves or reshapes
// either fails there instead of quietly computing every diff the slow way, or never cancelling.
import type * as MonacoNs from 'monaco-editor/editor/editor.api';
import { IDiffProviderFactoryService } from 'monaco-editor/editor/browser/widget/diffEditor/diffProviderFactoryService';
import { IEditorWorkerService } from 'monaco-editor/editor/common/services/editorWorker';
import { StandaloneServices } from 'monaco-editor/editor/standalone/browser/standaloneServices';
import type { DiffAlgorithm } from '../options';

type Model = MonacoNs.editor.ITextModel;

/** Monaco's IDocumentDiffProvider, as a diff view model uses it. */
interface DiffProvider {
  onDidChange: (listener: () => void) => { dispose(): void };
  computeDiff(original: Model, modified: Model, options: unknown, token: unknown): Promise<unknown>;
  dispose(): void;
}
interface DiffProviderFactory { createDiffProvider(options: { diffAlgorithm: DiffAlgorithm }): DiffProvider }

/** Each pair's algorithm, by its modified model (`setPairAlgorithm`). */
const algorithms = new WeakMap<Model, DiffAlgorithm>();
let installed = false;

/**
 * Has each diff computed with the algorithm `setPairAlgorithm` gave its pair, else the editor's
 * `diffAlgorithm` option. That option is the diff editor's, shared by every view model it makes:
 * flipping it per file would recompute the diff on screen too, in the one worker, ahead of the
 * next file's. Monaco's diff provider factory is wrapped so each view model's provider picks per
 * pair. Its cache of results (keyed by the models, not the algorithm) is unaffected: a pair always
 * gets the same algorithm.
 */
export function installPairAlgorithms(): void {
  if (installed) return;
  installed = true;
  const factory = StandaloneServices.get(IDiffProviderFactoryService) as DiffProviderFactory;
  const create = factory.createDiffProvider.bind(factory);
  // Defined, not assigned: the service is a delayed one, a Proxy that caches each method it hands
  // out on its own target and forwards an assignment to the instance behind it, where it'd go unseen.
  const wrapped: DiffProviderFactory['createDiffProvider'] = (options) => {
    const providers = new Map<DiffAlgorithm, DiffProvider>();
    const of = (algorithm: DiffAlgorithm) => {
      let p = providers.get(algorithm);
      if (!p) providers.set(algorithm, (p = create({ ...options, diffAlgorithm: algorithm })));
      return p;
    };
    const base = of(options.diffAlgorithm);
    return {
      onDidChange: base.onDidChange,
      computeDiff: (original, modified, opts, token) => of(algorithms.get(modified) ?? options.diffAlgorithm).computeDiff(original, modified, opts, token),
      dispose: () => { for (const p of providers.values()) p.dispose(); },
    };
  };
  Object.defineProperty(factory, 'createDiffProvider', { value: wrapped, configurable: true, writable: true });
}

/** The algorithm the pair with this modified model is computed with (`installPairAlgorithms`). */
export function setPairAlgorithm(modified: Model, algorithm: DiffAlgorithm): void {
  algorithms.set(modified, algorithm);
}

/** Monaco's WorkerManager: the editor worker's client, made again on the next request once null. */
interface WorkerManager { _editorWorkerClient: { dispose(): void } | null }

/**
 * Ends the editor worker, and what it's computing: the next request (a diff, links) starts a new
 * one, and syncs the models it needs to it again. A diff runs to its end in the worker (Monaco
 * can't cancel one), and the next file's waits for it: a lock file's took 4 s and more. Requests
 * still pending in the old worker never settle; each one's owner (a view model replaced, an editor's
 * link detection on a model it no longer shows) has moved on. False when Monaco's shape changed.
 */
export function restartEditorWorker(): boolean {
  const service = StandaloneServices.get(IEditorWorkerService) as { _workerManager?: WorkerManager };
  const manager = service._workerManager;
  if (!manager || !('_editorWorkerClient' in manager)) return false;
  manager._editorWorkerClient?.dispose();
  manager._editorWorkerClient = null;
  return true;
}
