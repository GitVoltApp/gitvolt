import { describe, expect, it } from 'vitest';
import type { BranchPicks } from '../api/gen/BranchPicks';
import type { SidebarPayload } from '../api/gen/SidebarPayload';
import type { ViewPayload } from '../api/gen/ViewPayload';
import { buildPanels, folderKey, sectionKey, sectionsOf, type Panel } from './model';

const branch = (name: string, tipTime: number, extra: object = {}) => ({ name, fullName: `refs/heads/${name}`, target: name.padEnd(40, '0'), upstream: null, ahead: 0, behind: 0, gone: false, tipTime, summary: `tip of ${name}`, author: 'Ada', isHead: false, worktree: null, checkedOut: null, pushTarget: null, pushBehind: null, rewritten: null, ...extra });
const payload: SidebarPayload = {
  locals: [branch('feature/login', 30), branch('feature/pay/v2', 10), branch('main', 20, { isHead: true })],
  remotes: [{ name: 'origin', host: null, hostKind: 'gitlab', branches: [{ name: 'main', fullName: 'refs/remotes/origin/main', target: 'm'.repeat(40), tipTime: 20, summary: 's', author: 'a' }] }],
  worktrees: [{ path: '/r', name: 'r', branch: 'main', head: 'h'.repeat(40), isMain: true, isCurrent: true, locked: false }],
  stashes: [{ index: 0, id: 's'.repeat(40), message: 'On main: wip', time: 5 }],
  tags: [{ name: 'v1.0', fullName: 'refs/tags/v1.0', target: 't'.repeat(40), time: 1 }],
  submodules: {},
};
const view = (panels: Panel[]) => panels.flatMap((p) => [`[${p.section.label} ${p.matched}${p.collapsed ? ' collapsed' : ''}]`, ...p.rows.map((r) => `${'  '.repeat(r.depth)}${r.type === 'folder' ? `${r.name}/` : r.label}`)]);
const opts = (o: Partial<Parameters<typeof buildPanels>[1]> = {}) => ({ filter: '', sort: {}, collapsed: new Set<string>(), ...o });

describe('sidebar model', () => {
  it('builds one panel per section, in fixed order, nesting by "/", folders and branches in one alphabetical order', () => {
    expect(view(buildPanels(sectionsOf(payload), opts()))).toEqual([
      '[Local 3]', '  feature/', '    login', '    pay/', '      v2', '  main',
      '[Remote 1]', '  origin/', '    main',
      '[Worktrees 1]', '  r',
      '[Stashes 1]', '  On main: wip',
      '[Tags 1]', '  v1.0',
    ]);
  });

  it('a branch sorts among the folders by name: dev comes before the f/ folder', () => {
    const mixed: SidebarPayload = { ...payload, locals: [branch('f/cart-stack', 1), branch('dev', 2), branch('backup/old', 3)] };
    expect(view(buildPanels(sectionsOf(mixed), opts())).slice(0, 6)).toEqual(['[Local 3]', '  backup/', '    old', '  dev', '  f/', '    cart-stack']);
  });

  it('the filter applies to every panel; counts are the matches, a panel with none shows 0', () => {
    const panels = buildPanels(sectionsOf(payload), opts({ filter: 'LOGIN', collapsed: new Set([folderKey('local', 'feature')]) }));
    expect(view(panels)).toEqual(['[Local 1]', '  feature/', '    login', '[Remote 0]', '[Worktrees 0]', '[Stashes 0]', '[Tags 0]']);
    expect(panels.map((p) => p.total)).toEqual([3, 1, 1, 1, 1]);
    expect(panels.every((p) => p.filtering)).toBe(true);
  });

  it('the filter also matches a remote branch by "remote/name"', () => {
    expect(buildPanels(sectionsOf(payload), opts({ filter: 'origin/ma' }))[1].matched).toBe(1);
  });

  it('a collapsed panel keeps its count but has no rows; collapsed folders hide their contents', () => {
    const panels = buildPanels(sectionsOf(payload), opts({ collapsed: new Set([folderKey('local', 'feature'), sectionKey('tags'), sectionKey('remote')]) }));
    expect(view(panels)).toEqual(['[Local 3]', '  feature/', '  main', '[Remote 1 collapsed]', '[Worktrees 1]', '  r', '[Stashes 1]', '  On main: wip', '[Tags 1 collapsed]']);
  });

  it('recent sort is flat and newest first; the remote keeps its group, rows unprefixed (K76)', () => {
    const panels = buildPanels(sectionsOf(payload), opts({ sort: { local: 'recent', remote: 'recent' } }));
    expect(view(panels).slice(0, 7)).toEqual(['[Local 3]', '  feature/login', '  main', '  feature/pay/v2', '[Remote 1]', '  origin/', '    main']);
  });

  it('K76: remote groups order by newest tip, branches inside newest first (ties by name), flat, no prefix', () => {
    const rb = (name: string, tipTime: number, remote: string) => ({ name, fullName: `refs/remotes/${remote}/${name}`, target: 'x'.repeat(40), tipTime, summary: '', author: '' });
    const p: SidebarPayload = { ...payload, remotes: [
      { name: 'origin', host: null, hostKind: 'generic', branches: [rb('old', 5, 'origin'), rb('b/deep', 10, 'origin'), rb('a', 10, 'origin')] },
      { name: 'upstream', host: null, hostKind: 'generic', branches: [rb('fresh', 50, 'upstream')] },
    ] };
    const panel = buildPanels(sectionsOf(p), opts({ sort: { remote: 'recent' } }))[1];
    expect(view([panel])).toEqual(['[Remote 4]', '  upstream/', '    fresh', '  origin/', '    a', '    b/deep', '    old']);
    const collapsed = buildPanels(sectionsOf(p), opts({ sort: { remote: 'recent' }, collapsed: new Set([folderKey('remote', 'upstream')]) }))[1];
    expect(view([collapsed])).toEqual(['[Remote 4]', '  upstream/', '  origin/', '    a', '    b/deep', '    old']);
  });

  describe('under a preset', () => {
    const states = (v?: ViewPayload) => sectionsOf(payload, v).flatMap((sec) => sec.items.map((it) => [it.name, it.presetState]));
    const view: ViewPayload = { trunk: ['main'], branches: [
      { name: 'feature/login', refs: ['refs/heads/feature/login'], state: 'hidden', by: { kind: 'otherwise' } },
      { name: 'main', refs: ['refs/heads/main', 'refs/remotes/origin/main'], state: 'folded', by: { kind: 'trunk' } },
    ] };

    it('hidden branches stay listed, struck through', () => {
      const rows = buildPanels(sectionsOf(payload, view), opts())[0].rows;
      expect(rows.some((r) => r.type === 'item' && r.item.name === 'feature/login' && r.item.presetState === 'hidden')).toBe(true);
      expect(buildPanels(sectionsOf(payload, view), opts())[0].matched).toBe(3);
    });

    it('folded branches say so', () => {
      expect(states(view)).toContainEqual(['main', 'folded']);
      // A remote branch shares its unit's state.
      expect(sectionsOf(payload, view)[1].items[0].presetState).toBe('folded');
    });

    const folders = (p: SidebarPayload, v?: ViewPayload, picks?: BranchPicks) => buildPanels(sectionsOf(p, v, picks), opts()).flatMap((panel) => panel.rows)
      .flatMap((r) => (r.type === 'folder' ? [[r.key, r.branchFolder, r.hidden]] : []));
    const states2 = (hidden: string[]): ViewPayload => ({ trunk: ['main'], branches: ['feature/login', 'feature/pay/v2', 'bot/x'].map((name) => ({ name, refs: [], state: hidden.includes(name) ? 'hidden' : 'shown', by: { kind: 'otherwise' } })) });
    const withBot: SidebarPayload = { ...payload, remotes: [{ ...payload.remotes[0], branches: [...payload.remotes[0].branches, { name: 'bot/x', fullName: 'refs/remotes/origin/bot/x', target: 'b'.repeat(40), tipTime: 1, summary: '', author: '' }] }], tags: [{ ...payload.tags[0], name: 'rel/v1' }] };

    it('a branch folder carries its prefix, and looks hidden when every branch under it is', () => {
      expect(folders(withBot, states2(['feature/login']))).toEqual([
        ['local:feature', 'feature/', false], ['local:feature/pay', 'feature/pay/', false],
        // A remote's own folder isn't a branch folder; its folders are, without the remote's name.
        ['remote:origin', undefined, undefined], ['remote:origin/bot', 'bot/', false],
        ['tags:rel', 'rel/', false],
      ]);
      expect(folders(withBot, states2(['feature/login', 'feature/pay/v2', 'bot/x'])).filter((f) => f[2]).map((f) => f[0])).toEqual(['local:feature', 'local:feature/pay', 'remote:origin/bot']);
      // Without a view nothing looks hidden; the folders can still be hidden.
      expect(folders(withBot).filter((f) => f[1]).every((f) => f[2] === false)).toBe(true);
    });

    it('a folder looks hidden when its folder pick, or one above it, hides it', () => {
      const hidden = (picks: { shown: string[]; hidden: string[] }) => folders(withBot, states2([]), picks).filter((f) => f[2]).map((f) => f[0]);
      expect(hidden({ shown: ['feature/pay/v2'], hidden: ['feature/pay/'] })).toEqual(['local:feature/pay']);
      expect(hidden({ shown: [], hidden: ['feature/', 'bot/'] })).toEqual(['local:feature', 'local:feature/pay', 'remote:origin/bot']);
      expect(hidden({ shown: ['feature/pay/'], hidden: ['feature/'] })).toEqual(['local:feature']);
    });

    it('a tag is shown or hidden, and a tag folder looks hidden by its tag picks or when every tag under it is', () => {
      const tags: SidebarPayload = { ...payload, tags: ['rel/v1', 'rel/v2', 'old/x', 'v1'].map((name) => ({ ...payload.tags[0], name, fullName: `refs/tags/${name}` })) };
      const v = (hiddenTags: string[]): ViewPayload => ({ ...states2([]), hiddenTags });
      const tagFolders = (vp: ViewPayload, picks?: BranchPicks) => folders(tags, vp, picks).filter((f) => String(f[0]).startsWith('tags:'));
      expect(sectionsOf(tags, v(['rel/v1']))[4].items.map((it) => [it.name, it.presetState])).toEqual([['rel/v1', 'hidden'], ['rel/v2', 'shown'], ['old/x', 'shown'], ['v1', 'shown']]);
      expect(tagFolders(v(['rel/v1']))).toEqual([['tags:old', 'old/', false], ['tags:rel', 'rel/', false]]);
      expect(tagFolders(v(['rel/v1', 'rel/v2']))).toEqual([['tags:old', 'old/', false], ['tags:rel', 'rel/', true]]);
      // A tag folder pick; a branch folder pick of the same name is a branch's.
      expect(tagFolders(v([]), { shown: [], hidden: ['rel/'], hiddenTags: ['old/'] })).toEqual([['tags:old', 'old/', true], ['tags:rel', 'rel/', false]]);
    });

    it('nothing changes without a view', () => {
      expect(states().every(([, st]) => st === undefined)).toBe(true);
      expect(sectionsOf(payload)).toEqual(sectionsOf(payload, null));
    });
  });
});
