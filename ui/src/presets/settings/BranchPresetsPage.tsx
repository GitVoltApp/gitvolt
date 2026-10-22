import { useEffect, useMemo, useState } from 'react';
import { create } from 'zustand';
import { api } from '../../api/client';
import type { PresetPreview } from '../../api/gen/PresetPreview';
import type { ViewSpec } from '../../api/gen/ViewSpec';
import { copyText, readClipboardText } from '../../api/transport';
import { useRuntime } from '../../app/runtime';
import { EMPTY_REPO_SETTINGS, useAppState } from '../../app/state';
import { fieldKey } from '../../fileColours/settings/FieldInput';
import { useSettingsPaste } from '../../settings/paste';
import type { SettingsCtx } from '../../settings/registry';
import { useKeys } from '../../ui/keyRouter';
import { useToast } from '../../ui/toastStore';
import { presetToJson } from '../json';
import { currentPreset } from '../model';
import { editingViews } from '../store';
import {
  deletePreset, duplicatePreset, entriesOf, forgetPick, homeOf, importInto, moveInGroup, newPreset, previewSpec, setInCycle, setScope, updatePreset, type PresetsDoc,
} from './edit';
import { PresetEditor } from './PresetEditor';
import { PresetList } from './PresetList';
import './presetsPage.css';

/** The preset the page shows, kept for the session; "Edit presets…" sets it to the current one. */
export const usePresetsPage = create<{ selected: string | null; focusName: string | null }>(() => ({ selected: null, focusName: null }));

const toast = (m: string) => useToast.getState().show(m);
export const newPresetId = (): string => crypto.randomUUID();
const PREVIEW_DEBOUNCE_MS = 150;

/** Writes what an edit changed: the app settings' presets, the repository's settings, or both;
 * then reloads the graphs whose view it changed. */
function save(before: PresetsDoc, after: PresetsDoc, repoPath: string | null): void {
  editingViews(() => {
    const app = useAppState.getState();
    if (after.settings !== before.settings) app.setSettings({ branchPresets: after.settings.branchPresets, builtinPresets: after.settings.builtinPresets });
    if (repoPath && after.repo && after.repo !== before.repo) app.updateRepo(repoPath, () => after.repo!);
  });
}

/** The live counts for `spec` over the tab's loaded window: asked 150 ms after the last edit, never polled. */
function usePreview(ctx: SettingsCtx, id: string, spec: ViewSpec | null): PresetPreview | null {
  const repoId = useRuntime((s) => (ctx.tabId ? s.tabs[ctx.tabId]?.repo?.id : undefined));
  const limit = useRuntime((s) => (ctx.tabId ? s.tabs[ctx.tabId]?.limit : null)) ?? null;
  const commitLimit = useAppState((s) => s.settings.commitLimit);
  const pin = useAppState((s) => (ctx.repoPath ? s.profile.repos[ctx.repoPath]?.pin : undefined)) ?? undefined;
  const key = spec ? JSON.stringify([spec, limit ?? commitLimit, pin]) : '';
  const [got, setGot] = useState<{ key: string; id: string; preview: PresetPreview } | null>(null);
  useEffect(() => {
    if (repoId === undefined || !spec) return;
    let live = true;
    const t = setTimeout(() => {
      api.branchPresetPreview(repoId, limit ?? commitLimit, pin, spec).then((preview) => { if (live) setGot({ key, id, preview }); }, () => {});
    }, PREVIEW_DEBOUNCE_MS);
    return () => { live = false; clearTimeout(t); };
    // `key` stands for spec, limit and pin.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [repoId, key, id]);
  // The last answer stays up while the next is asked (no flicker), but never another preset's.
  return got && got.id === id ? got.preview : null;
}

export function BranchPresetsPage({ ctx }: { ctx: SettingsCtx }) {
  const settings = useAppState((s) => s.settings);
  const rs = useAppState((s) => (ctx.repoPath ? s.profile.repos[ctx.repoPath] : undefined));
  const doc = useMemo<PresetsDoc>(() => ({ settings, repo: ctx.repoPath ? rs ?? EMPTY_REPO_SETTINGS : null }), [settings, rs, ctx.repoPath]);
  const entries = useMemo(() => entriesOf(doc), [doc]);
  const { selected, focusName } = usePresetsPage();
  const entry = entries.find((e) => e.preset.id === selected) ?? currentPreset(entries, rs);
  const p = entry.preset;
  const repoName = ctx.repoPath ? ctx.repoName ?? '' : null;

  const select = (id: string, focus = false) => usePresetsPage.setState({ selected: id, focusName: focus ? id : null });
  const apply = (next: PresetsDoc) => save(doc, next, ctx.repoPath);

  const spec = useMemo(() => previewSpec(p, doc.repo, settings.collapsePlacement), [p, doc.repo, settings.collapsePlacement]);
  const preview = usePreview(ctx, p.id, ctx.repoPath ? spec : null);

  const paste = (text: string) => {
    const id = newPresetId();
    const got = importInto(doc, text, id);
    if (!got) { toast("That isn't a GitVolt branch preset"); return; }
    apply(got.doc);
    select(got.id);
    toast(`Added ${got.name}`);
  };
  useSettingsPaste(paste);
  // Settings' dialog takes every key before React's handlers see it: Enter and Esc in a field come
  // through the key router (the rules' and the list's Alt+↑/↓ in `useSortableKeys`).
  useKeys('menu', fieldKey);

  const edit = (fn: Parameters<typeof updatePreset>[2]) => {
    const got = updatePreset(doc, p.id, fn, newPresetId());
    apply(got.doc);
    if (got.id !== p.id) select(got.id);
  };

  return (
    <div className="bp-page">
      <PresetList
        entries={entries}
        repoName={repoName}
        selected={p.id}
        onSelect={(id) => select(id)}
        onMove={(group, from, to) => apply(moveInGroup(doc, group, from, to))}
        onCycle={(id, on) => apply(setInCycle(doc, id, on))}
        onNew={() => { const id = newPresetId(); apply(newPreset(doc, id)); select(id, true); }}
        onPaste={() => { void readClipboardText().then(paste, () => toast("Couldn't read the clipboard")); }}
      />
      <PresetEditor
        key={p.id}
        entry={entry}
        repoName={repoName}
        picks={doc.repo?.branchPicks?.[p.id] ?? null}
        preview={preview}
        focusName={focusName === p.id}
        onEdit={edit}
        onScope={(scope) => { const got = setScope(doc, p.id, scope, newPresetId()); apply(got.doc); if (got.id !== p.id) select(got.id); }}
        onForget={(name, which) => apply(forgetPick(doc, p.id, name, which))}
        onDuplicate={() => { const id = newPresetId(); apply(duplicatePreset(doc, p.id, id)); select(id, true); }}
        onCopy={() => { void copyText(presetToJson(p)).then(() => toast(`Copied ${p.name}`)); }}
        onDelete={() => {
          // The selection moves to the preset above it.
          const at = entries.findIndex((e) => e.preset.id === p.id);
          const next = deletePreset(doc, p.id);
          apply(next);
          const left = entriesOf(next);
          if (homeOf(next, p.id) === null) select(left[Math.max(0, Math.min(at - 1, left.length - 1))].preset.id);
        }}
      />
    </div>
  );
}
