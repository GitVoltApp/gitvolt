import { useStore } from 'zustand';
import { createStore } from 'zustand/vanilla';
import { useTabView } from '../app/tabStores';
import { displayedOrder } from '../files/fileListPrefs';
import { wipFileCount } from '../format/wip';
import { openWipIndex, sameFile, type RepoViewState, type RepoViewStore } from '../repo/store';
import type { ButtonView } from './registry';

/**
 * The toolbar's jump to the working directory changes (GitKraken's "1 file change in working
 * directory [View Change]" banner, as a button): the tab's active worktree's WIP row, selected
 * as a click on it would.
 */

type WipState = Pick<RepoViewState, 'graph' | 'repoPath' | 'selection'>;

/** The active worktree's changed files, as its WIP row counts them (0: no WIP row). */
export function openWipCount(s: WipState): number {
  const w = s.graph.rows[openWipIndex(s)]?.wip;
  return w ? wipFileCount(w) : 0;
}

/** The active worktree's WIP row is the selection. */
export function onOpenWip(s: WipState): boolean {
  const i = openWipIndex(s);
  return i >= 0 && s.selection.kind === 'wip' && s.selection.index === i;
}

/** Runs `go` once the selected WIP's lists have settled (now, if they're held), or never if the
 * selection moves off that worktree first. */
function whenListed(store: RepoViewStore, worktree: string, go: () => void) {
  const here = (s: RepoViewState) => s.selection.kind === 'wip' && s.selection.worktree === worktree;
  const settled = (s: RepoViewState) => s.sections.every((sec) => sec.list.status !== 'loading');
  if (settled(store.getState())) return go();
  const off = store.subscribe((s) => {
    if (here(s) && !settled(s)) return;
    off();
    if (here(s)) go();
  });
}

/**
 * Selects the active worktree's WIP row. The open file stays open when the WIP changes it too
 * (by either path of a rename), switched to the WIP's side of it: unstaged before staged, as the
 * file list puts them, in the same view. Any other open file closes, so the WIP's lists show.
 * Nothing happens while that row is already selected.
 */
export function jumpToWip(store: RepoViewStore): void {
  const first = store.getState();
  if (openWipIndex(first) < 0 || onOpenWip(first)) return;
  first.leaveThen(() => {
    const s = store.getState();
    const index = openWipIndex(s);
    if (index < 0) return;
    const open = s.diff;
    s.selectRow(index);
    const selection = store.getState().selection;
    if (selection.kind !== 'wip') return;
    if (!open) return store.getState().setFocus('graph');
    whenListed(store, selection.worktree, () => {
      const st = store.getState();
      // The user opened something meanwhile: theirs wins.
      if (st.diff) return;
      for (const sec of st.sections) {
        if (sec.list.status !== 'ready') continue;
        const targets = displayedOrder(sec.list.data.files, sec.spec);
        const i = targets.findIndex((t) => sameFile(t, open));
        if (i < 0) continue;
        st.openFile({ ...targets[i], view: open.view }, targets.slice(i + 1, i + 2));
        return st.setFocus('files');
      }
      st.setFocus('graph');
    });
  });
}

const NO_STORE = createStore(() => ({ graph: { rows: [] }, repoPath: '', selection: { kind: 'none' } })) as unknown as RepoViewStore;

/** The button's caption, and whether it shows (any changes) and is pressed (the WIP is selected).
 * Hidden, it keeps its place: Actions and Search never move. */
export function useWipButtonView(tabId: string): ButtonView {
  const store = useTabView(tabId)?.store ?? NO_STORE;
  const count = useStore(store, openWipCount);
  const pressed = useStore(store, onOpenWip);
  return { label: `${count} ${count === 1 ? 'change' : 'changes'}`, disabled: false, hidden: count === 0, pressed };
}
