/** A diff's two text models, and what they were made for: the file, its language and both texts. */
export interface ModelPair<M> { path: string; language: string; originalText: string; modifiedText: string; original: M; modified: M }

/** Both sides' characters. */
export const pairChars = (p: ModelPair<unknown>): number => p.originalText.length + p.modifiedText.length;

/**
 * The model pairs of diffs shown lately, kept so showing one again (a reopen, a tab switched back
 * to) finds Monaco's computed diff, which it caches by the models. The least recently shown go
 * first, past `maxPairs` pairs or `maxChars` characters; a pair over `maxChars` alone isn't kept.
 * Pairs are only kept unedited: what's taken is shown as made.
 */
export class PairCache<M extends { dispose(): void }> {
  private pairs: ModelPair<M>[] = [];
  private readonly maxPairs: number;
  private readonly maxChars: number;

  constructor(maxPairs: number, maxChars: number) {
    this.maxPairs = maxPairs;
    this.maxChars = maxChars;
  }

  /** The pair made for this file, language and texts, out of the cache; null when there's none. */
  take(path: string, language: string, original: string, modified: string): ModelPair<M> | null {
    const i = this.pairs.findIndex((p) => p.path === path && p.language === language && p.originalText === original && p.modifiedText === modified);
    return i < 0 ? null : this.pairs.splice(i, 1)[0]!;
  }

  /** Keeps `pair` as the most recent, letting the oldest go past the limits. */
  put(pair: ModelPair<M>): void {
    this.pairs.push(pair);
    while (this.pairs.length > this.maxPairs) this.drop();
    this.trim(this.maxChars);
  }

  /** Lets the oldest pairs go until they hold at most `maxChars` characters. */
  trim(maxChars: number): void {
    while (this.pairs.length && this.chars() > maxChars) this.drop();
  }

  chars(): number {
    return this.pairs.reduce((n, p) => n + pairChars(p), 0);
  }

  get size(): number {
    return this.pairs.length;
  }

  private drop(): void {
    const p = this.pairs.shift()!;
    p.original.dispose();
    p.modified.dispose();
  }
}
