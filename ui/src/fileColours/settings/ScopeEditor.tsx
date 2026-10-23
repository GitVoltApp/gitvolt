import { Switch } from '../../ui/Switch';
import type { FileScope } from '../store';
import { TintSwatches } from './TintSwatches';

/** An open scope's second line: its colour and the folder option. Each change saves at once. */
export function ScopeEditor({ scope, onChange }: { scope: FileScope; onChange(next: FileScope): void }) {
  return (
    <div className="fc-editor">
      <span className="fc-editor-label">Colour</span>
      <TintSwatches value={scope.tint} onChange={(tint) => onChange({ ...scope, tint })} />
      <Switch label="Folders start collapsed in tree view" checked={scope.startCollapsed} onChange={(startCollapsed) => onChange({ ...scope, startCollapsed })} />
    </div>
  );
}
