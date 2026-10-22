import { Check, ExternalLink, GitMerge, GitPullRequestArrow, RefreshCw } from 'lucide-react';
import { activeTab, registerActions } from '../../app/actions';
import { lentHandler } from '../../app/lent';
import { isTypingTarget } from '../../ui/keys';
import { forgeOf } from '../mrStore';
import { refreshListNow } from '../poll';

/**
 * The open MR/PR view's main actions from the keyboard: Approve (Ctrl+Shift+A), Merge
 * (Ctrl+Shift+M), Open in browser (Ctrl+Shift+O) and the merge box's branch update (its split
 * button's main way, Ctrl+Alt+U). The view lends them while they apply
 * (`useMrActions`, `MergeBox`, `MrView`); Approve and Merge still ask first (a popover, as for
 * any confirm started from the keyboard); an update doesn't (nothing is lost: the forge keeps the
 * old head). Not while typing a comment. Refresh is palette-only (no key is an obvious
 * fit); it asks the forge for the open MR/PR again, and its list's twin works without one open.
 */
const lent = (id: string) => ({ when: () => lentHandler(id) !== null, run: () => lentHandler(id)?.(), yieldsTo: isTypingTarget });

const off = registerActions([
  { id: 'mr.approve', label: 'Approve the MR/PR', group: 'Repository', section: 'Merge request', icon: Check, tooltip: 'Approve the open merge request or pull request', shortcuts: ['Mod+Shift+A'], menu: false, ...lent('mr.approve') },
  { id: 'mr.merge', label: 'Merge the MR/PR', group: 'Repository', section: 'Merge request', icon: GitMerge, tooltip: 'Merge the open merge request or pull request', shortcuts: ['Mod+Shift+M'], menu: false, ...lent('mr.merge') },
  { id: 'mr.updateBranch', label: "Update the MR/PR's branch", group: 'Repository', section: 'Merge request', icon: GitPullRequestArrow, tooltip: "Rebase or update the open merge request's or pull request's branch with its target", shortcuts: ['Mod+Alt+U'], menu: false, ...lent('mr.updateBranch') },
  { id: 'mr.openInBrowser', label: 'Open the MR/PR in the browser', group: 'Repository', section: 'Merge request', icon: ExternalLink, tooltip: 'Open the open merge request or pull request on its forge', shortcuts: ['Mod+Shift+O'], menu: false, ...lent('mr.openInBrowser') },
  { id: 'mr.refresh', label: 'Refresh the MR/PR', group: 'Repository', section: 'Merge request', icon: RefreshCw, tooltip: 'Ask the forge for the open merge request or pull request again', menu: false, ...lent('mr.refresh') },
  { id: 'mr.refreshList', label: 'Refresh the MR/PR list', group: 'Repository', section: 'Merge request', icon: RefreshCw, tooltip: "Ask the forge for the repository's merge requests or pull requests again", menu: false, when: () => { const t = activeTab(); return t !== null && forgeOf(t.id).kind !== null; }, run: () => { const t = activeTab(); if (t) void refreshListNow(t.id); } },
]);
import.meta.hot?.dispose(off);
