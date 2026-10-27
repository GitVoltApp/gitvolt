// --- 4B T11 ---
import type { ForgeKind } from '../api/gen/ForgeKind';
import type { ForgeMr } from '../api/gen/ForgeMr';
// --- end 4B T11 ---
import type { BranchPicks } from '../api/gen/BranchPicks';
import type { BranchState } from '../api/gen/BranchState';
import type { HostKind } from '../api/gen/HostKind';
import type { LocalBranch } from '../api/gen/LocalBranch';
import type { RemoteBranch } from '../api/gen/RemoteBranch';
import type { SidebarPayload } from '../api/gen/SidebarPayload';
import type { SortMode } from '../api/gen/SortMode';
import type { StashItem } from '../api/gen/StashItem';
import type { TagItem } from '../api/gen/TagItem';
import type { ViewPayload } from '../api/gen/ViewPayload';
import type { WorktreeItem } from '../api/gen/WorktreeItem';
import { folderPickHides, tagPicksOf } from '../presets/model';

interface Base { key: string; name: string; target: string | null; time: number; /** A branch's state under the open preset (a tag's: shown or hidden); absent without a view. */ presetState?: BranchState }
export type SideItem =
  | (Base & { kind: 'local'; branch: LocalBranch })
  | (Base & { kind: 'remote'; remote: string; branch: RemoteBranch })
  | (Base & { kind: 'worktree'; worktree: WorktreeItem })
  | (Base & { kind: 'stash'; stash: StashItem })
  | (Base & { kind: 'tag'; tag: TagItem })
  // --- 4B T11 ---
  | (Base & { kind: 'mr'; mr: ForgeMr; forge: ForgeKind });
  // --- end 4B T11 ---

export type SectionKind = 'local' | 'remote' | 'mrs' | 'worktrees' | 'stashes' | 'tags';
export interface Section {
  id: SectionKind; kind: SectionKind; label: string; hosts?: Record<string, HostKind>; hostNames?: Record<string, string | null>; items: SideItem[]; nests: boolean; /** The empty body's text (default "Nothing here"). */ empty?: string;
  /** A branch panel's picks under the open preset (its folder picks), the Tags panel's tag picks
   * (`tagPicksOf`); absent without a view. */
  picks?: BranchPicks;
}

/** The sidebar's panels, in fixed display order (spec §6.4): Local, Remote (one top-level folder
 * per remote), Worktrees, Stashes, Tags. `picks`: the open preset's, for its hidden folders. */
export function sectionsOf(p: SidebarPayload, view?: ViewPayload | null, picks?: BranchPicks | null): Section[] {
  // Branch units are keyed by short name, so a local and its remotes share a state.
  const states = view ? new Map(view.branches.map((b) => [b.name, b.state])) : null;
  const st = (name: string): { presetState?: BranchState } => { const s = states?.get(name); return s ? { presetState: s } : {}; };
  const hosts: Record<string, HostKind> = {};
  const hostNames: Record<string, string | null> = {};
  for (const g of p.remotes) { hosts[g.name] = g.hostKind; hostNames[g.name] = g.host; }
  const branchPicks = view ? { picks: picks ?? { shown: [], hidden: [] } } : {};
  // Tags: hidden by hand only, so every tag is shown or hidden; their picks as branch-shaped lists.
  const hiddenTags = new Set(view?.hiddenTags ?? []);
  const tagState = (name: string): { presetState?: BranchState } => (view ? { presetState: hiddenTags.has(name) ? 'hidden' : 'shown' } : {});
  const tagPicks = view ? { picks: tagPicksOf(picks) } : {};
  return [
    { id: 'local', kind: 'local', label: 'Local', nests: true, ...branchPicks, items: p.locals.map((b): SideItem => ({ key: b.fullName, kind: 'local', name: b.name, target: b.target, time: b.tipTime, branch: b, ...st(b.name) })) },
    {
      id: 'remote', kind: 'remote', label: 'Remote', nests: true, hosts, hostNames, ...branchPicks,
      items: p.remotes.flatMap((g) => g.branches.map((b): SideItem => ({ key: b.fullName, kind: 'remote', name: b.name, target: b.target, time: b.tipTime, remote: g.name, branch: b, ...st(b.name) }))),
    },
    { id: 'worktrees', kind: 'worktrees', label: 'Worktrees', nests: false, items: p.worktrees.map((w): SideItem => ({ key: `wt:${w.path}`, kind: 'worktree', name: w.name, target: w.head, time: 0, worktree: w })) },
    { id: 'stashes', kind: 'stashes', label: 'Stashes', nests: false, items: p.stashes.map((s): SideItem => ({ key: `stash:${s.index}`, kind: 'stash', name: s.message, target: s.id, time: s.time, stash: s })) },
    { id: 'tags', kind: 'tags', label: 'Tags', nests: true, ...tagPicks, items: p.tags.map((t): SideItem => ({ key: t.fullName, kind: 'tag', name: t.name, target: t.target, time: t.time, tag: t, ...tagState(t.name) })) },
  ];
}

export type FlatRow =
  | {
    type: 'folder'; key: string; name: string; depth: number; collapsed: boolean; section: Section; hostKind?: HostKind; host?: string | null; remote?: string;
    /** A branch or tag folder's prefix (`deployed-commit/`, a remote's name left out), what hiding it picks. */
    branchFolder?: string;
    /** It looks hidden under the open preset: its folder pick hides it, or everything under it is hidden. */
    hidden?: boolean;
  }
  | { type: 'item'; key: string; item: SideItem; depth: number; label: string; section: Section };

/** One stacked panel: its header numbers and its (virtualized) body rows. `matched` is what the
 * filter leaves (== `total` when not filtering) and is the count the header shows. */
export interface Panel { section: Section; collapsed: boolean; matched: number; total: number; sort: SortMode; filtering: boolean; rows: FlatRow[] }

export interface RowOptions { filter: string; sort: Record<string, SortMode | undefined>; collapsed: ReadonlySet<string> }

export const sectionKey = (id: string) => `section:${id}`;
export const folderKey = (sectionId: string, path: string) => `${sectionId}:${path}`;

interface Node { folders: Map<string, Node>; items: SideItem[] }
const byName = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: 'base' });
/** What the filter matches against, and what flat (recent) lists show: remote branches carry
 * their remote's name. */
const fullName = (it: SideItem) => (it.kind === 'remote' ? `${it.remote}/${it.name}` : it.name);

/** A branch or tag panel's folders (by tree path, a remote's name first): how many items are under
 * each, at any depth, and how many of them the preset hides. Unfiltered. */
function hiddenCounts(s: Section): Map<string, { all: number; hidden: number }> {
  const out = new Map<string, { all: number; hidden: number }>();
  for (const it of s.items) {
    if (it.kind !== 'local' && it.kind !== 'remote' && it.kind !== 'tag') continue;
    const dirs = it.name.split('/').slice(0, -1);
    let path = '';
    for (const part of it.kind === 'remote' ? [it.remote, ...dirs] : dirs) {
      path = path ? `${path}/${part}` : part;
      const c = out.get(path) ?? { all: 0, hidden: 0 };
      c.all += 1;
      if (it.presetState === 'hidden') c.hidden += 1;
      out.set(path, c);
    }
  }
  return out;
}

/**
 * Builds the sidebar's panels (spec §6.4). A case-insensitive substring filter applies to every
 * panel, keeping and forcing open the parent folders of any match (a panel's count is then the
 * match count; a panel with none shows 0 and an empty body). `tree` sort (the default) nests
 * items by `/` in their name, folders first, both sorted case-insensitively (remote branches
 * nest under a folder per remote); `recent` sort is flat, newest `time` first. Only nesting
 * panels (Local, Remote, Tags) sort. A collapsed panel (or folder, unless filtering forces it
 * open) contributes no rows below it.
 */
export function buildPanels(sections: Section[], o: RowOptions): Panel[] {
  const q = o.filter.trim().toLowerCase();
  return sections.map((s): Panel => {
    const matched = q ? s.items.filter((i) => fullName(i).toLowerCase().includes(q)) : s.items;
    const sort: SortMode = s.nests ? o.sort[s.id] ?? 'tree' : 'tree';
    const collapsed = o.collapsed.has(sectionKey(s.id));
    const panel: Panel = { section: s, collapsed, matched: matched.length, total: s.items.length, sort, filtering: !!q, rows: [] };
    if (collapsed) return panel;
    const out = panel.rows;
    const item = (it: SideItem, depth: number, label = it.name): FlatRow => ({ type: 'item', key: it.key, item: it, depth, label, section: s });
    if (!s.nests) {
      for (const it of matched) out.push(item(it, 1));
      return panel;
    }
    if (sort === 'recent' && s.kind === 'remote') {
      // K76: still one group per remote (no `origin/` on its rows), flat inside, newest tip
      // first; the groups themselves ordered by their newest tip.
      const groups = new Map<string, SideItem[]>();
      for (const it of matched) if (it.kind === 'remote') groups.set(it.remote, [...(groups.get(it.remote) ?? []), it]);
      const newest = (items: SideItem[]) => Math.max(...items.map((i) => i.time));
      const ordered = [...groups].sort(([an, a], [bn, b]) => newest(b) - newest(a) || byName(an, bn));
      for (const [remote, items] of ordered) {
        const key = folderKey(s.id, remote);
        const fc = !q && o.collapsed.has(key);
        out.push({ type: 'folder', key, name: remote, depth: 1, collapsed: fc, section: s, remote, hostKind: s.hosts?.[remote], host: s.hostNames?.[remote] });
        if (fc) continue;
        for (const it of [...items].sort((a, b) => b.time - a.time || byName(a.name, b.name))) out.push(item(it, 2));
      }
      return panel;
    }
    if (sort === 'recent') {
      for (const it of [...matched].sort((a, b) => b.time - a.time || byName(a.name, b.name))) out.push(item(it, 1, fullName(it)));
      return panel;
    }
    const counts = s.picks ? hiddenCounts(s) : null;
    const root: Node = { folders: new Map(), items: [] };
    for (const it of matched) {
      let n = root;
      const dirs = it.name.split('/').slice(0, -1);
      for (const part of it.kind === 'remote' ? [it.remote, ...dirs] : dirs) {
        if (!n.folders.has(part)) n.folders.set(part, { folders: new Map(), items: [] });
        n = n.folders.get(part)!;
      }
      n.items.push(it);
    }
    // One alphabetical list per level, folders and branches mixed (`backup/`,
    // `dev`, `f/`), not folders first.
    const walk = (n: Node, depth: number, prefix: string) => {
      const entries: ({ folder: string; child: Node } | { leaf: SideItem; name: string })[] = [
        ...[...n.folders].map(([folder, child]) => ({ folder, child })),
        ...n.items.map((leaf) => ({ leaf, name: leaf.name.split('/').pop()! })),
      ];
      entries.sort((a, b) => byName('folder' in a ? a.folder : a.name, 'folder' in b ? b.folder : b.name));
      for (const e of entries) {
        if ('leaf' in e) {
          out.push(item(e.leaf, depth, e.name));
          continue;
        }
        const path = prefix ? `${prefix}/${e.folder}` : e.folder;
        const key = folderKey(s.id, path);
        const fc = !q && o.collapsed.has(key);
        const isRemote = s.kind === 'remote' && depth === 1;
        const branchFolder = s.kind === 'local' || s.kind === 'tags' ? `${path}/` : s.kind === 'remote' && !isRemote ? `${path.slice(path.indexOf('/') + 1)}/` : undefined;
        const c = counts?.get(path);
        const hidden = branchFolder && s.picks ? folderPickHides(s.picks, branchFolder) || (!!c && c.hidden === c.all) : false;
        out.push({
          type: 'folder', key, name: e.folder, depth, collapsed: fc, section: s,
          ...(isRemote ? { remote: e.folder, hostKind: s.hosts?.[e.folder], host: s.hostNames?.[e.folder] } : {}),
          ...(branchFolder ? { branchFolder, hidden } : {}),
        });
        if (!fc) walk(e.child, depth + 1, path);
      }
    };
    walk(root, 1, '');
    return panel;
  });
}
