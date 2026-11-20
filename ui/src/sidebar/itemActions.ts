import type { LucideIcon } from 'lucide-react';
import { create } from 'zustand';
import type { RepoViewStore } from '../repo/store';
import type { SectionKind, SideItem } from './model';

/** Sidebar double-clicks by item kind (a branch row checks it out: a
 * worktree row becomes the active one), registered by the features that own them. */
export interface SidebarCtx { tabId: string; store: RepoViewStore }
const doubleClicks = new Map<SideItem['kind'], (ctx: SidebarCtx, item: SideItem) => void>();

export function registerSidebarDoubleClick(kind: SideItem['kind'], fn: (ctx: SidebarCtx, item: SideItem) => void): () => void {
  doubleClicks.set(kind, fn);
  return () => { if (doubleClicks.get(kind) === fn) doubleClicks.delete(kind); };
}

export function sidebarDoubleClick(ctx: SidebarCtx, item: SideItem): boolean {
  const fn = doubleClicks.get(item.kind);
  if (!fn) return false;
  fn(ctx, item);
  return true;
}

/** A section header's icon buttons (the Worktrees header's +). */
export interface HeaderAction { id: string; icon: LucideIcon; label: string; run: (ctx: { tabId: string }) => void }
export const useHeaderActions = create<{ bySection: Partial<Record<SectionKind, HeaderAction[]>> }>(() => ({ bySection: {} }));

export function registerSidebarHeaderAction(section: SectionKind, a: HeaderAction): () => void {
  useHeaderActions.setState((s) => ({ bySection: { ...s.bySection, [section]: [...(s.bySection[section] ?? []), a] } }));
  return () => useHeaderActions.setState((s) => ({ bySection: { ...s.bySection, [section]: (s.bySection[section] ?? []).filter((x) => x !== a) } }));
}

export const sidebarHeaderActions = (section: SectionKind): HeaderAction[] => useHeaderActions.getState().bySection[section] ?? [];
