/**
 * The tokens of the lines a content change makes, from those of the first and last lines it
 * replaces (as (end offset, metadata) pairs): the text before and after the change keeps its
 * tokens, and the inserted text takes one next to it. They show until the worker's exact tokens
 * for those lines arrive. `undefined` (no tokens known) gives `undefined` lines: plain.
 */
export function editTokens(first: Uint32Array | undefined, last: Uint32Array | undefined, startCol: number, endCol: number, parts: string[]): Array<Uint32Array | undefined> {
  if (!first || !last) return parts.map(() => undefined);
  let at = first[first.length - 1];
  const head: number[] = [];
  for (let j = 0, prev = 0; j < first.length && prev < startCol; prev = first[j], j += 2) {
    const end = Math.min(first[j], startCol);
    if (end > prev) head.push(end - prev, first[j + 1]);
  }
  // Typed text continues the token it follows (as Monaco's store has it); text that replaces some
  // takes the style of what it replaced.
  const replaces = first !== last || endCol > startCol;
  for (let j = 0; j < first.length; j += 2) {
    if (first[j] > startCol || (!replaces && first[j] === startCol)) { at = first[j + 1]; break; }
  }
  const tail: number[] = [];
  for (let j = 0, prev = 0; j < last.length; prev = last[j], j += 2) {
    const start = Math.max(prev, endCol);
    if (last[j] > start) tail.push(last[j] - start, last[j + 1]);
  }
  return parts.map((p, i) => ends([...(i === 0 ? head : []), p.length, at, ...(i === parts.length - 1 ? tail : [])], at));
}

/** (length, metadata) runs → (end offset, metadata) pairs, empty runs dropped and equal neighbours
 * merged; an empty line keeps one token. */
function ends(runs: number[], fallback: number): Uint32Array {
  const out: number[] = [];
  let end = 0;
  for (let j = 0; j < runs.length; j += 2) {
    if (runs[j] === 0) continue;
    end += runs[j];
    if (out.length > 0 && out[out.length - 1] === runs[j + 1]) out[out.length - 2] = end;
    else out.push(end, runs[j + 1]);
  }
  return Uint32Array.from(out.length > 0 ? out : [0, fallback]);
}
