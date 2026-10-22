import { Palette } from 'lucide-react';
import { useAppState } from '../app/state';
import type { FileTarget, FolderTarget, MenuEnv } from '../menu/menuEnv';
import { registerMenu } from '../menu/registry';
import type { MenuRow } from '../menu/types';
import { canOpenFileColours, requestAddPattern } from './addRequest';
import { matcherFor } from './match';
import { folderPatternOf, typePatternOf } from './patterns';
import { scopesOf } from './store';
import { tintHex } from './tint';

/** The "Colour …" item: only while a File colours page can open to receive the pattern. */
function addItem(id: string, label: string, pattern: string, commonDir: string): MenuRow[] {
  if (!canOpenFileColours()) return [];
  return [{
    kind: 'action', id, label, icon: Palette,
    tooltip: `Add ${pattern} to a scope in Settings › File colours`,
    run: () => requestAddPattern({ commonDir, pattern, via: label }),
  }];
}

/** `Scope: <name>` for the scope that colours `path` (a folder: the scope covering all of it). */
function scopeHeader(id: string, path: string, isFolder: boolean, commonDir: string): MenuRow[] {
  const scopes = scopesOf(useAppState.getState().profile, commonDir);
  if (!scopes.length) return [];
  const m = matcherFor(scopes);
  if (isFolder) {
    const scope = scopes[m.folderScopeOf(path)];
    return scope ? [{ kind: 'header', id, label: `Scope: ${scope.name}`, swatch: tintHex(scope.tint) ?? undefined }] : [];
  }
  const hit = m.explain(path);
  const scope = hit && scopes[hit.scope];
  return hit && scope ? [{ kind: 'header', id, label: `Scope: ${scope.name}`, detail: hit.pattern, swatch: tintHex(scope.tint) ?? undefined }] : [];
}

const offs = [
  registerMenu<FileTarget, MenuEnv>({
    id: 'file.colour', kind: 'file', group: 'colour', order: 0,
    when: (_t, env) => !!env.commonDir,
    rows: (t, env) => {
      const commonDir = env.commonDir;
      if (!commonDir) return [];
      const type = typePatternOf(t.path);
      const folder = folderPatternOf(t.path, false);
      return [
        ...scopeHeader('file.colour.scope', t.path, false, commonDir),
        ...addItem('file.colour.type', `Colour ${type.label} files…`, type.pattern, commonDir),
        ...(folder ? addItem('file.colour.folder', `Colour folder ${folder.label}…`, folder.pattern, commonDir) : []),
      ];
    },
  }),
  registerMenu<FolderTarget, MenuEnv>({
    id: 'folder.colour', kind: 'folder', group: 'colour', order: 0,
    when: (_t, env) => !!env.commonDir,
    rows: (t, env) => {
      const commonDir = env.commonDir;
      if (!commonDir) return [];
      const folder = folderPatternOf(t.path, true);
      return [
        ...scopeHeader('folder.colour.scope', t.path, true, commonDir),
        ...(folder ? addItem('folder.colour.folder', `Colour folder ${folder.label}…`, folder.pattern, commonDir) : []),
      ];
    },
  }),
];

import.meta.hot?.dispose(() => offs.forEach((off) => off()));
