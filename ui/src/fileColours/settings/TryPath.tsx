import { useState } from 'react';
import { matcherFor } from '../match';
import type { FileScope } from '../store';
import './fileColoursSettings.css';

/** Which scope (and pattern) a path lands in, as you type it. */
export function TryPath({ scopes }: { scopes: readonly FileScope[] }) {
  const [path, setPath] = useState('');
  const p = path.trim();
  const hit = p ? matcherFor(scopes).explain(p) : null;
  const scope = hit ? scopes[hit.scope] : null;
  return (
    <div className="fc-try">
      <input aria-label="Try a path" placeholder="Try a path, e.g. src/api/user.test.ts" value={path} spellCheck={false} onChange={(e) => setPath(e.target.value)} />
      {/* A reserved line: the answer appearing doesn't move what's below. */}
      <div className="fc-try-out" aria-live="polite">
        {p && (scope && hit ? `→ ${scope.name} (pattern ${scope.patterns.indexOf(hit.pattern) + 1}, ${hit.pattern})` : '→ No scope')}
      </div>
    </div>
  );
}
