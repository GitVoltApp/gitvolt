import { matcherFor } from './match';
import type { FileScope } from './store';

const CHUNK = 5000;

const abortError = () => new DOMException('Aborted', 'AbortError');

/** Per scope, the paths it takes (first match). Chunks of 5000 with a `setTimeout(0)` yield between them. */
export async function countScopes(scopes: readonly FileScope[], paths: readonly string[], signal?: AbortSignal): Promise<number[]> {
  const counts = scopes.map(() => 0);
  const m = matcherFor(scopes);
  for (let i = 0; i < paths.length; i += CHUNK) {
    if (signal?.aborted) throw abortError();
    const end = Math.min(i + CHUNK, paths.length);
    for (let j = i; j < end; j++) {
      const s = m.scopeOf(paths[j]);
      if (s >= 0) counts[s]++;
    }
    if (end < paths.length) await new Promise<void>((r) => setTimeout(r, 0));
  }
  if (signal?.aborted) throw abortError();
  return counts;
}
