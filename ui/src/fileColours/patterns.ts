import { escapeGlob } from './glob';

/** The pattern for a file's type: its last extension (`*.ts` for `a.test.ts`); a name with no
 * extension, a dotfile or a trailing dot stands for itself. The label is the pattern unescaped. */
export function typePatternOf(path: string): { pattern: string; label: string } {
  const name = path.slice(path.lastIndexOf('/') + 1);
  const dot = name.lastIndexOf('.');
  if (dot <= 0 || dot === name.length - 1) return { pattern: escapeGlob(name), label: name };
  const ext = name.slice(dot + 1);
  return { pattern: `*.${escapeGlob(ext)}`, label: `*.${ext}` };
}

/** The pattern covering a folder: `<dir>/**`, the file's own folder or the folder itself; null
 * for a file at the root. */
export function folderPatternOf(path: string, isFolder: boolean): { pattern: string; label: string } | null {
  const dir = isFolder ? path : path.slice(0, Math.max(0, path.lastIndexOf('/')));
  if (!dir) return null;
  return { pattern: `${escapeGlob(dir)}/**`, label: `${dir}/` };
}
