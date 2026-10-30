import { Check } from 'lucide-react';
import { useAppState } from '../../app/state';
import { copyText, readClipboardText } from '../../api/transport';
import { openMenuAt } from '../../menu/menuStore';
import { pressOpener } from '../../menu/pressDrag';
import type { MenuRow } from '../../menu/types';
import { useToast } from '../../ui/toastStore';
import { appendScopes, parseScopesJson, scopesToJson } from '../json';
import { repoLabel } from '../scopes';
import type { FileScope } from '../store';
import './fileColoursSettings.css';

const toast = (m: string) => useToast.getState().show(m);
const plural = (n: number) => `${n} scope${n === 1 ? '' : 's'}`;

/** Appends the scopes in `text` (GitVolt file colours JSON); anything else only shows a toast. */
export function pasteScopes(text: string, scopes: readonly FileScope[], onChange: (next: FileScope[]) => void): void {
  const incoming = parseScopesJson(text);
  if (!incoming || incoming.length === 0) { toast("That isn't GitVolt file colours"); return; }
  onChange(appendScopes(scopes, incoming));
  toast(`Added ${plural(incoming.length)}`);
}

export function ShareBar({ commonDir, scopes, onChange }: { commonDir: string; scopes: readonly FileScope[]; onChange(next: FileScope[]): void }) {
  const fileColours = useAppState((s) => s.profile.fileColours);
  const others = Object.entries(fileColours).filter(([dir, list]) => dir !== commonDir && list.length > 0);
  const copy = () => { void copyText(scopesToJson(scopes)).then(() => toast(`Copied ${plural(scopes.length)}`)); };
  const paste = () => { void readClipboardText().then((t) => pasteScopes(t, scopes, onChange), () => toast("Couldn't read the clipboard")); };
  const from = (el: Element) => {
    const rows = (): MenuRow[] => others.map(([dir, list]) => ({
      kind: 'action', id: `fc.from.${dir}`, label: `${repoLabel(dir)} (${plural(list.length)})`, icon: Check, tooltip: `Add ${repoLabel(dir)}'s scopes to this repository`,
      run: () => { onChange(appendScopes(scopes, list)); toast(`Added ${plural(list.length)}`); },
    }));
    openMenuAt(el, rows(), undefined, rows, 'Copy from another repository');
  };
  return (
    <div className="fc-share">
      <button type="button" className="fc-btn" disabled={scopes.length === 0} onClick={copy}>Copy as JSON</button>
      <button type="button" className="fc-btn" onClick={paste}>Paste scopes</button>
      <button type="button" className="fc-btn" aria-haspopup="menu" disabled={others.length === 0}
        title={others.length === 0 ? 'No other repository has scopes' : undefined} {...pressOpener(from)}>
        Copy from another repository…
      </button>
    </div>
  );
}
