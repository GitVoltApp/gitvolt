// @vitest-environment happy-dom
// The Monaco internals the diff host relies on (diffWork.ts), against the installed Monaco. If this
// fails after a Monaco upgrade, its diff provider factory or editor worker manager moved or changed
// shape: every diff would take the slow algorithm, or a superseded one would block the next.
import { describe, expect, it } from 'vitest';
import * as monaco from 'monaco-editor/editor/editor.api';
import { IDiffProviderFactoryService } from 'monaco-editor/editor/browser/widget/diffEditor/diffProviderFactoryService';
import { IEditorWorkerService } from 'monaco-editor/editor/common/services/editorWorker';
import { StandaloneServices } from 'monaco-editor/editor/standalone/browser/standaloneServices';
import { installPairAlgorithms, restartEditorWorker, setPairAlgorithm } from './diffWork';

type Model = monaco.editor.ITextModel;
interface Provider { computeDiff(o: Model, m: Model, options: object, token: object): Promise<{ moves: unknown[]; changes: unknown[] }>; dispose(): void }
const factory = () => StandaloneServices.get(IDiffProviderFactoryService) as { createDiffProvider(o: { diffAlgorithm: string }): Provider };
const token = { isCancellationRequested: false, onCancellationRequested: () => ({ dispose() {} }) };
const options = { ignoreTrimWhitespace: false, maxComputationTimeMs: 0, computeMoves: true, extendToSubwords: false };

/** A block of lines that moved from the top to the bottom: `advanced` reports the move, `legacy` finds no moves. */
function movedPair(): [Model, Model] {
  const block = Array.from({ length: 6 }, (_, i) => `const moved${i} = computeSomething(${i}, 'a long enough line');`);
  const rest = Array.from({ length: 20 }, (_, i) => `let stays${i} = ${i};`);
  return [monaco.editor.createModel([...block, ...rest].join('\n'), 'plaintext'), monaco.editor.createModel([...rest, ...block].join('\n'), 'plaintext')];
}

describe("Monaco's diff internals", () => {
  it('computes each pair with the algorithm set for it, else the one the provider was made with', async () => {
    installPairAlgorithms();
    const provider = factory().createDiffProvider({ diffAlgorithm: 'advanced' });
    const [o1, m1] = movedPair();
    const [o2, m2] = movedPair();
    setPairAlgorithm(m2, 'legacy');
    const advanced = await provider.computeDiff(o1, m1, options, token);
    const legacy = await provider.computeDiff(o2, m2, options, token);
    expect(advanced.moves.length).toBeGreaterThan(0);
    expect(legacy.moves).toEqual([]);
    expect(legacy.changes.length).toBeGreaterThan(0);
    provider.dispose();
  });

  it('restarts the editor worker: its client goes, and the next request makes a new one', async () => {
    installPairAlgorithms();
    const manager = () => (StandaloneServices.get(IEditorWorkerService) as { _workerManager: { _editorWorkerClient: object | null } })._workerManager;
    const provider = factory().createDiffProvider({ diffAlgorithm: 'advanced' });
    const [o, m] = movedPair();
    await provider.computeDiff(o, m, options, token);
    const client = manager()._editorWorkerClient;
    expect(client).not.toBeNull();
    expect(restartEditorWorker()).toBe(true);
    expect(manager()._editorWorkerClient).toBeNull();
    const [o2, m2] = movedPair();
    expect((await provider.computeDiff(o2, m2, options, token)).moves.length).toBeGreaterThan(0);
    expect(manager()._editorWorkerClient).not.toBeNull();
    expect(manager()._editorWorkerClient).not.toBe(client);
    provider.dispose();
  });
});
