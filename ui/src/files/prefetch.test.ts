import { describe, expect, it } from 'vitest';
import type { FileChange } from '../api/gen/FileChange';
import { targetFor } from '../repo/store';
import { PREFETCH_MAX_LINES, worthPrefetching } from './prefetch';

const spec = { kind: 'commit' as const, id: 'c'.repeat(40), parent: 0 };
const change = (path: string, additions: number | null, deletions: number | null = 0, oldPath: string | null = null): FileChange => ({ path, oldPath, status: oldPath ? 'R' : 'M', additions, deletions, old: { kind: 'object', oid: 'a'.repeat(40) }, new: { kind: 'object', oid: 'b'.repeat(40) }, submodule: false });
const worth = (c: FileChange, withChange = true) => worthPrefetching(targetFor(c, spec), withChange ? c : undefined);

describe('worthPrefetching', () => {
  it('skips images, PDFs and media by extension, either side of a rename', () => {
    for (const p of ['shot.PNG', 'art.svg', 'docs/manual.pdf', 'song.mp3', 'clip.mov', 'font.woff2']) expect(worth(change(p, 1), false), p).toBe(false);
    expect(worth(change('notes.txt', 1, 0, 'old/manual.pdf'), false)).toBe(false);
    expect(worth(change('src/app.ts', 1), false)).toBe(true);
    expect(worth(change('Makefile', 1), false)).toBe(true);
  });

  it('with a numstat, skips a binary (no line counts) and a change over the line cap', () => {
    expect(worth(change('data.bin', null, null))).toBe(false);
    expect(worth(change('big.json', PREFETCH_MAX_LINES - 10, 10))).toBe(true);
    expect(worth(change('big.json', PREFETCH_MAX_LINES, 1))).toBe(false);
  });
});
