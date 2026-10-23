import { useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { useStore } from 'zustand';
import { createStore } from 'zustand/vanilla';
import { useAppState } from '../../app/state';
import { useTabView } from '../../app/tabStores';
import { openWorktree, type RepoViewStore } from '../../repo/store';
import type { SettingsCtx } from '../../settings/registry';
import { useSettingsPaste } from '../../settings/paste';
import { useKeys } from '../../ui/keyRouter';
import { useSortable } from '../../ui/sortable/useSortable';
import { countScopes } from '../counts';
import { moveScope, newScope } from '../scopes';
import { STARTER_SCOPES } from '../starters';
import { newScopeId, scopesOf, setScopes, useTabCommonDir, type FileScope } from '../store';
import { AddStrip } from './AddStrip';
import { fieldKey } from './FieldInput';
import { ScopeRow } from './ScopeRow';
import { pasteScopes, ShareBar } from './ShareBar';
import { TryPath } from './TryPath';
import './fileColoursSettings.css';

/** Stands in for a tab's view store until its first graph has loaded. */
const EMPTY_STORE = createStore(() => ({ panel: null })) as unknown as RepoViewStore;
type Counts = { changed: number[] | null; repo: number[] | null };
const NONE: Counts = { changed: null, repo: null };
const STARTERS = ['Tests', 'Generated', 'Docs'];

/** The paths in the tab's details panel lists (what "changed" counts), and the open worktree's tracked files (null until loaded). */
function useCountedPaths(tabId: string | null) {
  const view = useTabView(tabId ?? '');
  const panel = useStore(view?.store ?? EMPTY_STORE, (s) => s.panel);
  const changed = useMemo(() => {
    const out = new Set<string>();
    for (const sec of panel?.sections ?? []) if (sec.list.status === 'ready') for (const f of sec.list.data.files) out.add(f.path);
    return [...out];
  }, [panel]);
  const [repo, setRepo] = useState<string[] | null>(null);
  const wt = view ? openWorktree(view.store.getState()) : null;
  useEffect(() => {
    setRepo(null);
    if (!view || !wt) return;
    let live = true;
    view.services.worktreeFiles.get(wt).then((p) => { if (live) setRepo(p); }, () => {});
    return () => { live = false; };
  }, [view, wt]);
  return { changed, repo };
}

export function FileColoursPage({ ctx }: { ctx: SettingsCtx }) {
  const commonDir = useTabCommonDir(ctx.tabId);
  if (!commonDir) return <p className="dim fc-empty-page">Open a repository to set its file colours.</p>;
  return <Page ctx={ctx} commonDir={commonDir} />;
}

function Page({ ctx, commonDir }: { ctx: SettingsCtx; commonDir: string }) {
  const scopes = useAppState((s) => scopesOf(s.profile, commonDir));
  const onChange = (next: FileScope[]) => setScopes(commonDir, next);
  const [openId, setOpenId] = useState<string | null>(null);
  const { changed, repo } = useCountedPaths(ctx.tabId);
  const [counts, setCounts] = useState<Counts>(NONE);
  const list = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const ac = new AbortController();
    setCounts((c) => ({ changed: null, repo: c.repo }));
    countScopes(scopes, changed, ac.signal).then((n) => setCounts((c) => ({ ...c, changed: n })), () => {});
    return () => ac.abort();
  }, [scopes, changed]);
  useEffect(() => {
    if (!repo) { setCounts((c) => ({ ...c, repo: null })); return; }
    const ac = new AbortController();
    setCounts((c) => ({ ...c, repo: null }));
    countScopes(scopes, repo, ac.signal).then((n) => setCounts((c) => ({ ...c, repo: n })), () => {});
    return () => ac.abort();
  }, [scopes, repo]);

  const sortable = useSortable({ count: scopes.length, axis: 'y', onMove: (from, to) => onChange(moveScope(scopes, from, to)), label: (i) => scopes[i]?.name ?? `Scope ${i + 1}` });

  // Settings' dialog takes every key before React sees it: Enter and Esc in a field, and Alt+↑/↓ on a
  // scope, come through the key router (the dialog's claim keeps them from running twice).
  useKeys('menu', (e) => {
    const f = fieldKey(e);
    if (f) return f;
    if (!e.altKey || e.ctrlKey || e.metaKey || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
    const item = e.target instanceof Element ? e.target.closest<HTMLElement>('[data-sortable-item]') : null;
    if (!item || item.parentElement !== list.current) return;
    const from = Number(item.dataset.sortableItem);
    sortable.itemProps(from).onKeyDown?.(e as unknown as ReactKeyboardEvent<HTMLElement>);
    e.preventDefault();
    return 'handled';
  });
  useSettingsPaste((text) => pasteScopes(text, scopes, onChange));

  const starter = (name: string) => {
    const s = STARTER_SCOPES.find((x) => x.name === name);
    if (s) onChange([...scopes, { ...s, patterns: [...s.patterns], id: newScopeId() }]);
  };
  const addScope = () => {
    const s = newScope(scopes, []);
    onChange([...scopes, s]);
    setOpenId(s.id);
  };

  return (
    <div className="fc-page">
      <div className="fc-strip-slot">
        <AddStrip commonDir={commonDir} scopes={scopes} repoPaths={repo} onChange={onChange} onCreated={setOpenId} />
      </div>
      {scopes.length === 0 ? (
        <div className="fc-starters">
          <span className="dim">No scopes yet. Start from one:</span>
          {STARTERS.map((n) => <button key={n} type="button" className="fc-btn" onClick={() => starter(n)}>{n}</button>)}
        </div>
      ) : (
        <div ref={list} className="fc-list" role="list" aria-label="Scopes" {...sortable.containerProps}>
          {scopes.map((s, i) => (
            <ScopeRow
              key={s.id}
              scope={s}
              open={openId === s.id}
              counts={{ changed: counts.changed?.[i] ?? null, repo: counts.repo?.[i] ?? null }}
              itemProps={sortable.itemProps(i)}
              handleProps={sortable.handleProps(i)}
              onToggle={() => setOpenId(openId === s.id ? null : s.id)}
              onChange={(next) => onChange(scopes.map((x) => (x.id === s.id ? next : x)))}
              onDelete={() => onChange(scopes.filter((x) => x.id !== s.id))}
            />
          ))}
          <span className="fc-sr" aria-live="polite">{sortable.liveText}</span>
        </div>
      )}
      <button type="button" className="fc-new" onClick={addScope}>+ New scope</button>
      <TryPath scopes={scopes} />
      <ShareBar commonDir={commonDir} scopes={scopes} onChange={onChange} />
    </div>
  );
}
