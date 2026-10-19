import type { CodeTokens } from '../diff/monaco/shiki';

/** A code block's tokens, tokenized in the highlight worker, in the order blocks ask (its chunk,
 * and Shiki's, load with the first). A cancelled block is dropped from the worker's queue. */
export function queueHighlight(code: string, lang: string): { result: Promise<CodeTokens | null>; cancel(): void } {
  let cancelled = false;
  let job: { cancel(): void } | null = null;
  const result = import('../diff/highlight/codeTokens').then(({ codeTokens }) => {
    if (cancelled) return new Promise<never>(() => {});
    const j = codeTokens(code, lang);
    job = j;
    return j.result;
  });
  return { result, cancel: () => { cancelled = true; job?.cancel(); } };
}
