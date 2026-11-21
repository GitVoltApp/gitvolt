import { ArchiveRestore, PackageOpen, PackagePlus, Trash2 } from 'lucide-react';
import type { StashPart } from '../api/gen/StashPart';
import { StashIcon } from '../icons/stash';
import { activeRuntime, activeTab, registerActions } from '../app/actions';
import type { RepoCtx } from '../app/repoContext';
import { useRuntime, type TabRuntime } from '../app/runtime';
import { tabStore } from '../app/tabStores';
import type { CommitTarget, FileTarget, FolderTarget, MenuEnv, SidebarTarget, WipTarget } from '../menu/menuEnv';
import { registerMenu } from '../menu/registry';
import type { MenuRow } from '../menu/types';
import { openWipFile } from '../stage/keyActions';
import { COMMIT_QUEUED, stagingKey, useStaging } from '../stage/store';
import { registerToolbarButton, type ButtonView } from '../toolbar/registry';
import type { WriteCtx } from '../write/client';
import { isTypingTarget } from '../ui/keys';
import { stashLabel } from '../sidebar/stashLabel';
import { addToStashFor, applyStash, dropStash, stashPathsFor, stashPushFor } from './actions';

const writeCtx = (): WriteCtx | null => {
  const t = activeTab();
  const rt = activeRuntime();
  return t?.kind === 'repo' && rt?.repo ? { tabId: t.id, repoId: rt.repo.id, worktree: rt.worktree ?? rt.repo.path } : null;
};
const newest = (rt: TabRuntime | null | undefined) => rt?.sidebar?.stashes[0] ?? null;
/** The active worktree has a WIP row: something to stash. */
const dirty = (rt: TabRuntime | null | undefined) => {
  const wt = rt?.worktree ?? rt?.repo?.path;
  return !!wt && !!rt?.graph?.rows.some((r) => r.wip?.worktreePath === wt);
};

const rows = (ctx: WriteCtx, oid: string): MenuRow[] => [
  { kind: 'action', id: 'stash.apply', label: 'Apply', icon: PackageOpen, tooltip: 'Apply the stash and keep it (staged changes come back staged)', run: () => { void applyStash(ctx, oid, false); } },
  { kind: 'action', id: 'stash.pop', label: 'Pop', icon: ArchiveRestore, tooltip: 'Apply the stash, then delete it (you can undo this)', run: () => { void applyStash(ctx, oid, true); } },
  { kind: 'action', id: 'stash.drop', label: 'Delete', icon: Trash2, tooltip: 'Delete the stash (you can undo this)', run: () => { void dropStash(ctx, oid); } },
];

/** Why a WIP file's or folder's Stash can't run now: a queued commit, or a merge, rebase… in its
 * worktree (the write refuses then), as its neighbours (Stage, Discard) say. */
export function stashPathsBlocked(ctx: WriteCtx): string | undefined {
  if (useStaging.getState().committing[stagingKey(ctx.repoId, ctx.worktree)]) return COMMIT_QUEUED;
  const op = tabStore(ctx.tabId)?.getState().graph.worktrees.find((w) => w.path === ctx.worktree)?.inProgress;
  return op ? `Finish or abort the ${op} first` : undefined;
}

const stashPathsRow = (ctx: WriteCtx, id: string, label: string, tooltip: string, paths: string[], part: StashPart, blocked?: string): MenuRow =>
  ({ kind: 'action', id, label, icon: StashIcon, tooltip, run: () => { void stashPathsFor(ctx, paths, part); }, disabledReason: blocked ?? stashPathsBlocked(ctx) });

/** A stash's name in a menu row: its own message, a long one cut short. */
export const shortStashName = (message: string): string => {
  const text = stashLabel(message).text;
  return text.length > 32 ? `${text.slice(0, 31).trimEnd()}…` : text;
};

/** Add to stash, beside a WIP file's or folder's Stash: the same part joins the latest stash (none:
 * no row). `what`: the label's middle ("staged changes in src/"). */
const addRow = (ctx: WriteCtx, id: string, what: string, tooltip: string, paths: string[], part: StashPart, blocked?: string): MenuRow[] => {
  const top = newest(useRuntime.getState().tabs[ctx.tabId]);
  if (!top) return [];
  const name = stashLabel(top.message).text;
  return [{
    kind: 'action', id, label: `Add ${what} to "${shortStashName(top.message)}"`, icon: PackagePlus,
    tooltip: `${tooltip} to the latest stash, "${name}", which keeps its name and its place (you can undo this)`,
    run: () => { void addToStashFor(ctx, paths, part, top.id); }, disabledReason: blocked ?? stashPathsBlocked(ctx),
  }];
};

/** The part a WIP list's Stash takes: the list's own. */
const partOf = (staged: boolean | undefined): StashPart => (staged ? 'staged' : 'unstaged');
/** What a part's Stash leaves: the other list's changes. */
const OTHER = { staged: 'unstaged', unstaged: 'staged' } as const;

const stashView = ({ tabId }: RepoCtx): ButtonView =>
  dirty(useRuntime((s) => s.tabs[tabId])) ? { tooltip: 'Stash every change, named from the WIP message', disabled: false } : { tooltip: 'No changes to stash', disabled: true };
const popView = ({ tabId }: RepoCtx): ButtonView => {
  const top = newest(useRuntime((s) => s.tabs[tabId]));
  return top ? { tooltip: `Pop "${top.message}"`, disabled: false } : { tooltip: 'No stashes', disabled: true };
};

const offs = [
  registerActions([
    {
      id: 'stash.push', label: 'Stash', group: 'Repository', icon: StashIcon, tooltip: 'Stash every change, named from the WIP message', shortcuts: ['Mod+Alt+S'],
      // Ctrl+Alt is AltGr on Windows: typed text never runs it.
      yieldsTo: isTypingTarget,
      when: () => dirty(activeRuntime()),
      run: () => { const c = writeCtx(); if (c) void stashPushFor(c); },
    },
    {
      id: 'stash.pop', label: 'Pop', group: 'Repository', icon: ArchiveRestore, tooltip: 'Apply the newest stash and delete it', shortcuts: ['Mod+Alt+P'],
      yieldsTo: isTypingTarget,
      when: () => !!newest(activeRuntime()),
      // The oid is read at the click, so a Pop that waits in the queue acts on the stash clicked.
      run: () => { const c = writeCtx(); const s = newest(activeRuntime()); if (c && s) void applyStash(c, s.id, true); },
    },
    {
      // The open WIP file, as Stage file's Mod+Shift+S; Stash's own chord with Shift. The part is
      // the list's it was opened from, as its menu's Stash row.
      id: 'stash.file', label: 'Stash file', group: 'Repository', icon: StashIcon, tooltip: 'Stash the open file’s staged changes if it was opened from Staged, its unstaged ones from Unstaged', shortcuts: ['Mod+Alt+Shift+S'], menu: false,
      yieldsTo: isTypingTarget,
      when: () => { const o = openWipFile(); return !!o && !stashPathsBlocked(o.ctx); },
      run: () => { const o = openWipFile(); if (o) void stashPathsFor(o.ctx, [o.file.path], partOf(o.staged)); },
    },
  ]),
  registerToolbarButton({ action: 'stash.push', label: 'Stash', order: 40, useView: stashView }),
  registerToolbarButton({ action: 'stash.pop', label: 'Pop', order: 41, useView: popView }),
  registerMenu<SidebarTarget, MenuEnv>({
    id: 'sidebar.stash', kind: 'sidebar', group: 'stash', order: 0,
    when: (t, env) => t.what === 'stash' && !!env.write,
    rows: (t, env) => (t.what === 'stash' && env.write ? rows(env.write, t.sha) : []),
  }),
  registerMenu<CommitTarget, MenuEnv>({
    id: 'commit.stash', kind: 'commit', group: 'stash', order: 0,
    when: (t, env) => t.isStash && !!env.write,
    rows: (t, env) => (env.write ? rows(env.write, t.sha) : []),
  }),
  // A WIP file's or folder's Stash (Unstaged and Staged; Conflicted's are Mark resolved's): the
  // part of its changes in that list (Unstaged's untracked files too), the rest left as it is.
  registerMenu<FileTarget, MenuEnv>({
    id: 'file.stash', kind: 'file', group: 'stage', order: 1,
    when: (t, env) => !!t.wip && t.wip.status !== 'U' && !t.wip.submodule && !!env.write,
    rows: (t, env) => {
      const part = partOf(t.wip!.staged);
      const ctx = { ...env.write!, worktree: t.wip!.worktree };
      return [
        stashPathsRow(ctx, 'file.stash', `Stash ${part} changes`, `Stash this file’s ${part} changes; its ${OTHER[part]} changes and the other files stay as they are (you can undo this)`, [t.path], part),
        ...addRow(ctx, 'file.stashAdd', `${part} changes`, `Add this file’s ${part} changes`, [t.path], part),
      ];
    },
  }),
  registerMenu<FolderTarget, MenuEnv>({
    id: 'folder.stash', kind: 'folder', group: 'stage', order: 1,
    when: (t, env) => !!t.wip?.files?.some((f) => f.status !== 'U') && !!env.write,
    // git refuses a stash while a file is conflicted: one under the folder says so here.
    rows: (t, env) => {
      const conflicted = (t.wip!.conflicted.length > 0 || t.wip!.files!.some((f) => f.status === 'U')) ? `Resolve the conflicted files in ${t.path}/ first` : undefined;
      const part = partOf(t.wip!.staged);
      const what = part === 'staged' ? 'staged changes' : 'unstaged changes and untracked files';
      const ctx = { ...env.write!, worktree: t.wip!.worktree };
      return [
        stashPathsRow(ctx, 'folder.stash', `Stash ${part} changes in ${t.path}/`, `Stash the ${what} in ${t.path}/; their ${OTHER[part]} changes and the other files stay as they are (you can undo this)`, [t.path], part, conflicted),
        ...addRow(ctx, 'folder.stashAdd', `${part} changes in ${t.path}/`, `Add the ${what} in ${t.path}/`, [t.path], part, conflicted),
      ];
    },
  }),
  registerMenu<WipTarget, MenuEnv>({
    id: 'wip.stash', kind: 'wip', group: 'stash', order: 0,
    when: (_t, env) => !!env.write,
    rows: (t, env) => (env.write ? [{
      kind: 'action', id: 'wip.stash', label: 'Stash changes', icon: StashIcon,
      tooltip: `Stash every change in ${t.name ?? 'this worktree'}, named from its WIP message`,
      run: () => { void stashPushFor({ ...env.write!, worktree: t.worktree }); },
    }] : []),
  }),
];
import.meta.hot?.dispose(() => { for (const off of offs) off(); });
