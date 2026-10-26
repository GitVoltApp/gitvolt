import { api } from '../../api/client';
import { tabStore } from '../../app/tabStores';
import { useForge } from '../mrStore';

/**
 * An MR/PR head Compare fetched into a ref of its own (`refs/gitvolt/compare/…`) lives only while
 * that Compare needs it. Once the tab no longer compares the head (it leaves the compare or picks
 * another), the MR/PR view it was opened from closes, or the tab closes, `dropCompareRefs`
 * deletes it, and the graph stops drawing commits no branch holds (a merged MR's, its branch
 * deleted). One per tab, as the core keeps one per repository.
 */
interface Held { repo: number; head: string; shown: boolean; stop: () => void }

const held = new Map<string, Held>();

/** Tab `tabId` just fetched MR/PR `number`'s `head` into its ref, for a Compare in `repo`. Nothing
 * ends it before that compare shows; the fetch replaced any ref held before (the core's own
 * one-at-a-time), so that hold just stops. */
export function holdCompareRef(tabId: string, repo: number, number: number, head: string): void {
  held.get(tabId)?.stop();
  const store = tabStore(tabId);
  const comparing = (): boolean => {
    const sel = store?.getState().selection;
    return sel?.kind === 'compare' && (sel.from === head || sel.to === head);
  };
  const h: Held = { repo, head, shown: comparing(), stop: () => {} };
  const stops = [
    store?.subscribe((s, prev) => {
      if (s.selection === prev.selection) return;
      if (comparing()) h.shown = true;
      else if (h.shown) releaseCompareRef(tabId);
    }),
    useForge.subscribe((s, prev) => {
      if (prev.byTab[tabId]?.openMr === number && s.byTab[tabId]?.openMr !== number) releaseCompareRef(tabId);
    }),
  ];
  h.stop = () => stops.forEach((stop) => stop?.());
  held.set(tabId, h);
}

/** Ends tab `tabId`'s hold, if it has one, deleting the ref. `unshown`: only a hold of `head`
 * whose compare hasn't shown (a Compare that fetched it, then failed). */
export function releaseCompareRef(tabId: string, unshown?: string): void {
  const h = held.get(tabId);
  if (!h || (unshown !== undefined && (h.head !== unshown || h.shown))) return;
  h.stop();
  held.delete(tabId);
  // Housekeeping: one left by a failure goes at the repository's next open.
  api.dropCompareRefs(h.repo).catch(() => {});
}
