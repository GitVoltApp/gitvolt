import { existsSync, readFileSync } from 'node:fs';
import { gunzipSync } from 'node:zlib';
import { describe, expect, test } from 'vitest';
import type { GraphPayload } from '../../api/gen/GraphPayload';
import type { RepoSummary } from '../../api/gen/RepoSummary';
import type { RevealPayload } from '../../api/gen/RevealPayload';
import type { ViewSpec } from '../../api/gen/ViewSpec';
import { createRuntime } from '../runtime';
import { goldenData, goldenManifest, WASM_GZ } from '../testing';
import { createDemoTransport } from '../transport';
import { HANDLERS } from '.';

describe.skipIf(!existsSync(WASM_GZ))('the graph handler (needs `just demo-wasm`)', () => {
  test('lays basic out through the WebAssembly pipeline', async () => {
    const { loadGraphWasm } = await import('../engine/graph');
    const graph = await loadGraphWasm(gunzipSync(readFileSync(WASM_GZ)));
    const runtime = createRuntime({ manifest: goldenManifest(), data: goldenData(), graph });
    const t = createDemoTransport(Promise.resolve(runtime), HANDLERS);
    const { id } = (await t.call({ method: 'openRepo', params: { path: '/demo/basic' } })) as RepoSummary;
    const g = (await t.call({ method: 'graph', params: { repo: id, limit: null } })) as GraphPayload;
    expect(g.rows).toHaveLength(10);
    const commitIds = g.rows.filter((r) => r.kind !== 'wip').map((r) => r.id);
    const order = await runtime.ctx.repos.get(id).walkOrder(2000);
    expect(new Set(order)).toEqual(new Set(commitIds));
    expect(order).toHaveLength(commitIds.length);
    expect(g.rows.filter((r) => r.kind === 'wip').map((r) => r.id)).toEqual(['wip:/demo/basic', 'wip:/demo/.extra/basic/wt-hotfix']);
    expect(g.pinnedRefs).toEqual(['refs/heads/main', 'refs/remotes/origin/main']);
    expect(g.rows.some((r) => r.kind === 'stash')).toBe(true);
    const off = (await t.call({ method: 'graph', params: { repo: id, limit: null, pin: { kind: 'off' } } })) as GraphPayload;
    expect(off.pinnedRefs).toEqual([]);
    await expect(t.call({ method: 'graph', params: { repo: id, limit: null, active: '/elsewhere' } })).rejects.toMatchObject({ kind: 'InvalidInput', message: '/elsewhere is not a worktree of this repository' });
  });

  test('a view applies through the WebAssembly pipeline; its preview counts every branch; graphReveal lays out a fold and answers null for a stale key',async () => {
    const { loadGraphWasm } = await import('../engine/graph');
    const graph = await loadGraphWasm(gunzipSync(readFileSync(WASM_GZ)));
    const runtime = createRuntime({ manifest: goldenManifest(), data: goldenData(), graph });
    const t = createDemoTransport(Promise.resolve(runtime), HANDLERS);
    const { id } = (await t.call({ method: 'openRepo', params: { path: '/demo/basic' } })) as RepoSummary;
    const view: ViewSpec = { rules: [], otherwise: 'show', picks: { shown: [], hidden: [] }, foldMerged: false, ghostLines: true, trunkPatterns: [], expanded: [], placement: 'keepDateOrder' };
    const plain = (await t.call({ method: 'graph', params: { repo: id, limit: null } })) as GraphPayload;
    const viewed = (await t.call({ method: 'graph', params: { repo: id, limit: null, view } })) as GraphPayload;
    expect(viewed.rows).toEqual(plain.rows);
    expect(plain.view).toBeUndefined();
    const branches = viewed.view!.branches;
    expect(branches.length).toBeGreaterThan(0);
    expect(branches.every((b) => b.state === 'shown')).toBe(true);
    const tip = plain.rows.find((r) => r.kind === 'commit')!.id;
    expect(await t.call({ method: 'graphReveal', params: { repo: id, limit: null, view, key: 'branch:feature/login', below: false } })).toBeNull();
    expect(await t.call({ method: 'graphHiddenOwner', params: { repo: id, limit: null, view, sha: tip } })).toBeNull();
    expect(await t.call({ method: 'branchPresetPreview', params: { repo: id, limit: null, view } })).toEqual({ rules: [branches.length], shown: branches.length, collapsed: 0, hidden: 0, folded: 0 });
    // Folding feature/login into its merge on main: two rows fewer, the merge marked with its tail
    // in lane 1, and a folded commit names the merge that holds it.
    const fold: ViewSpec = { ...view, foldMerged: true };
    const folded = (await t.call({ method: 'graph', params: { repo: id, limit: null, view: fold } })) as GraphPayload;
    expect(folded.rows).toHaveLength(plain.rows.length - 2);
    const merge = folded.rows.find((r) => r.fold)!;
    expect(merge.fold).toEqual({ key: `fold:${merge.id}`, branch: 'feature/login', count: 2, tailLane: 1 });
    const gone = plain.rows.find((r) => r.summary === 'Login form')!;
    expect(await t.call({ method: 'graphHiddenOwner', params: { repo: id, limit: null, view: fold, sha: gone.id } })).toBe(`fold:${merge.id}`);
    // Its reveal panel: the folded commits beside the trunk's lane, the trunk rows tagged.
    const panel = (await t.call({ method: 'graphReveal', params: { repo: id, limit: null, view: fold, key: `fold:${merge.id}`, below: true } })) as RevealPayload;
    expect(panel).toMatchObject({ key: `fold:${merge.id}`, branch: 'feature/login', count: 2 });
    expect(panel.entryLane).toBe(merge.fold!.tailLane);
    const login = panel.rows.find((r) => r.id === gone.id)!;
    expect(login.lane).toBe(panel.entryLane);
    expect(panel.tags.every((tag) => panel.rows[tag.row].lane === merge.lane)).toBe(true);
  });

  test('find searches hidden commits too under a view', async () => {
    const { loadGraphWasm } = await import('../engine/graph');
    const graph = await loadGraphWasm(gunzipSync(readFileSync(WASM_GZ)));
    const runtime = createRuntime({ manifest: goldenManifest(), data: goldenData(), graph });
    const t = createDemoTransport(Promise.resolve(runtime), HANDLERS);
    const { id } = (await t.call({ method: 'openRepo', params: { path: '/demo/basic' } })) as RepoSummary;
    const plain = (await t.call({ method: 'graph', params: { repo: id, limit: null } })) as GraphPayload;
    const loginForm = plain.rows.find((r) => r.summary === 'Login form')!.id;
    const fold: ViewSpec = { rules: [], otherwise: 'show', picks: { shown: [], hidden: [] }, foldMerged: true, ghostLines: false, trunkPatterns: [], expanded: [], placement: 'keepDateOrder' };
    const folded = (await t.call({ method: 'graph', params: { repo: id, limit: null, view: fold } })) as GraphPayload;
    expect(folded.rows.some((r) => r.id === loginForm)).toBe(false);
    expect(await t.call({ method: 'findText', params: { repo: id, query: 'login form' } })).toEqual([loginForm]);
  });
});
