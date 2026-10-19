import type { WipPayload } from '../api/gen/WipPayload';

/** A WIP row's change counts, `✎n +n −n ⚠n`, zeros left out (spec §8.6). Shared by the graph's
 * WIP row and the details panel's WIP header. */
export function wipCountsText(w: WipPayload): string {
  const parts: [number, string][] = [[w.modified, '✎'], [w.added, '+'], [w.deleted, '−'], [w.renamed, '➜'], [w.conflicted, '⚠']];
  return parts.filter(([n]) => n > 0).map(([n, sign]) => `${sign}${n}`).join(' ');
}

/** How many files a WIP row changes: each changed path once, staged or not, untracked and
 * conflicted ones included (the WIP panel's "N file changes" before its lists load). */
export function wipFileCount(w: WipPayload): number {
  return w.modified + w.added + w.deleted + w.renamed + w.conflicted;
}
