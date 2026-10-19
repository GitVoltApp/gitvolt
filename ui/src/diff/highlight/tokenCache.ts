/** A text's tokens, a line each ((end offset, metadata) pairs; never changed once stored). */
export type Lines = ReadonlyArray<Uint32Array | undefined>;

/** Per stored line: the array object and the slot that holds it. */
const LINE_OVERHEAD = 48;
const bytesOf = (lines: Lines) => lines.reduce((n, l) => n + LINE_OVERHEAD + (l?.byteLength ?? 0), 0);

/**
 * The tokens of recently highlighted texts, by language, theme and content, so reopening a version
 * or going between its diff and File View colours it at once. Least recently used goes first, past
 * `maxBytes` or `maxEntries`; a text over a quarter of `maxBytes` isn't kept.
 */
export class TokenCache {
  private entries = new Map<string, { lines: Lines; bytes: number }>();
  private bytes = 0;

  private readonly maxBytes: number;
  private readonly maxEntries: number;

  constructor(maxBytes: number, maxEntries: number) {
    this.maxBytes = maxBytes;
    this.maxEntries = maxEntries;
  }

  get(key: string): Lines | undefined {
    const e = this.entries.get(key);
    if (!e) return undefined;
    this.entries.delete(key);
    this.entries.set(key, e);
    return e.lines;
  }

  set(key: string, lines: Lines): void {
    const bytes = bytesOf(lines);
    this.delete(key);
    if (bytes > this.maxBytes / 4) return;
    this.entries.set(key, { lines, bytes });
    this.bytes += bytes;
    for (const [k] of this.entries) {
      if (this.bytes <= this.maxBytes && this.entries.size <= this.maxEntries) break;
      this.delete(k);
    }
  }

  private delete(key: string): void {
    const e = this.entries.get(key);
    if (!e) return;
    this.entries.delete(key);
    this.bytes -= e.bytes;
  }

  get size(): number { return this.entries.size; }
}
