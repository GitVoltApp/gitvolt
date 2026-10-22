import { describe, expect, it } from 'vitest';
import { folderPatternOf, typePatternOf } from './patterns';

describe('typePatternOf', () => {
  it.each([
    ['a/b/market.snap', '*.snap', '*.snap'],
    ['src/a.test.ts', '*.ts', '*.ts'],
    ['Makefile', 'Makefile', 'Makefile'],
    ['x/.gitignore', '.gitignore', '.gitignore'],
    ['x/archive.', 'archive.', 'archive.'],
    ['x/a.t[s', '*.t\\[s', '*.t[s'],
  ])('%s', (path, pattern, label) => {
    expect(typePatternOf(path)).toEqual({ pattern, label });
  });
});

describe('folderPatternOf', () => {
  it("covers the file's folder", () => {
    expect(folderPatternOf('crates/c/tests/snapshots/market.snap', false)).toEqual({ pattern: 'crates/c/tests/snapshots/**', label: 'crates/c/tests/snapshots/' });
  });
  it('is null for a root file', () => expect(folderPatternOf('Cargo.lock', false)).toBeNull());
  it('escapes a folder', () => expect(folderPatternOf('a[1]/b', true)?.pattern).toBe('a\\[1\\]/b/**'));
});
