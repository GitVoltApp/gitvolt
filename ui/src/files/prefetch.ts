import type { FileChange } from '../api/gen/FileChange';
import { IMAGE_MIME } from '../image/sources';
import { mediaOf } from '../media/kind';
import type { DiffTarget } from '../repo/store';

/** Above this many changed lines, a neighbour's contents aren't fetched ahead of the step to it. */
export const PREFETCH_MAX_LINES = 2000;

const extOf = (path: string) => {
  const name = path.slice(path.lastIndexOf('/') + 1);
  const dot = name.lastIndexOf('.');
  return dot < 0 ? '' : name.slice(dot + 1).toLowerCase();
};

/** An image, a PDF or media: shown from its bytes, often megabytes of base64. */
const heavyKind = (path: string) => extOf(path) in IMAGE_MIME || extOf(path) === 'pdf' || mediaOf(path) !== null;

/**
 * Whether the file after (or before) the open one is worth fetching ahead of ↑/↓: a prefetch is
 * a guess, and a 10 KB file's open shouldn't pull a 1 MB neighbour, or a PDF's 6 MB. No image,
 * PDF or media (by either path of a rename); with `change`'s numstat, no binary (size unknown)
 * and nothing over `PREFETCH_MAX_LINES` changed lines.
 */
export function worthPrefetching(t: DiffTarget, change?: FileChange | null): boolean {
  if (heavyKind(t.path) || (t.oldPath !== null && heavyKind(t.oldPath))) return false;
  if (!change) return true;
  if (change.additions === null || change.deletions === null) return false;
  return change.additions + change.deletions <= PREFETCH_MAX_LINES;
}
