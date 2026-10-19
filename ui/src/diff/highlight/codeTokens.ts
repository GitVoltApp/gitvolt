import { currentEditorTheme } from '../../theme/editorThemes';
import { bundledLanguages } from '../shikiLanguages';
import { highlightClient } from './client';
import type { CodeTokens } from './protocol';

/** A Markdown code block's tokens in the current editor theme, from the highlight worker; `null`
 * for a language Shiki doesn't ship (the block stays plain). A cancelled block never resolves. */
export function codeTokens(code: string, lang: string): { result: Promise<CodeTokens | null>; cancel(): void } {
  const id = lang.toLowerCase();
  if (!Object.hasOwn(bundledLanguages, id)) return { result: Promise.resolve(null), cancel: () => {} };
  let cancelled = false;
  let job: { cancel(): void } | null = null;
  const result = highlightClient().then((c) => {
    if (cancelled) return new Promise<never>(() => {});
    const j = c.code(code, id, currentEditorTheme());
    job = j;
    return j.result;
  });
  return { result, cancel: () => { cancelled = true; job?.cancel(); } };
}
