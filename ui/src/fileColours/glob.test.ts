import { describe, expect, it } from 'vitest';
import { compilePattern, escapeGlob, matchesPattern } from './glob';

const table: [string, string, boolean][] = [
  ['*.snap', 'a/b/c.snap', true], ['*.snap', 'c.snap', true], ['*.snap', 'c.snapx', false],
  ['**/tests/**', 'tests/a.rs', true], ['**/tests/**', 'x/tests/a.rs', true], ['**/tests/**', 'x/tests', false], ['**/tests/**', 'x/testsuite/a', false],
  ['db/migrations/**', 'db/migrations/0042.sql', true], ['db/migrations/**', 'x/db/migrations/1.sql', false],
  ['/Cargo.lock', 'Cargo.lock', true], ['/Cargo.lock', 'a/Cargo.lock', false],
  ['Cargo.lock', 'a/Cargo.lock', true],
  ['docs', 'docs/a.md', true], ['docs', 'x/docs/a.md', true], ['docs', 'docsy/a.md', false],
  ['docs/', 'docs/a.md', true], ['docs/', 'docs', false],
  ['src/*.rs', 'src/a.rs', true], ['src/*.rs', 'src/x/a.rs', false],
  ['src/**/*.rs', 'src/a.rs', true], ['src/**/*.rs', 'src/x/y/a.rs', true],
  ['?.md', 'a.md', true], ['?.md', 'ab.md', false],
  ['[ab].txt', 'a.txt', true], ['[ab].txt', 'c.txt', false],
  ['[!ab].txt', 'c.txt', true],
  ['a+b(c).txt', 'a+b(c).txt', true], ['a+b(c).txt', 'aab(c).txt', false],
  ['\\!important.txt', '!important.txt', true],
  ['README.md', 'readme.md', false],
];

describe('glob', () => {
  it('compilePattern matches like .gitignore', () => {
    for (const [pattern, path, expected] of table) {
      expect(matchesPattern(pattern, path), `${pattern} vs ${path}`).toBe(expected);
    }
  });

  it('compilePattern returns null for blanks, comments, a lone ! or /, and an unclosed [', () => {
    for (const p of ['', '   ', '# x', '!', '/', '[abc']) expect(compilePattern(p), JSON.stringify(p)).toBeNull();
  });

  it('a leading ! makes an exclude', () => {
    expect(compilePattern('!*.md')!.exclude).toBe(true);
    expect(compilePattern('*.md')!.exclude).toBe(false);
  });

  it('escapeGlob matches only the literal text', () => {
    expect(escapeGlob('a[1]/*.txt')).toBe('a\\[1\\]/\\*.txt');
    expect(matchesPattern(escapeGlob('a[1]/*.txt'), 'a[1]/*.txt')).toBe(true);
    expect(matchesPattern(escapeGlob('a[1]/*.txt'), 'a1/x.txt')).toBe(false);
    expect(matchesPattern(escapeGlob('#x'), '#x')).toBe(true);
    expect(matchesPattern(escapeGlob('!x'), '!x')).toBe(true);
  });

  it('never throws on odd input', () => {
    for (const p of ['[z-a]', '[]', '[^]', 'a\\', '**', '***/x', '(', '\\', '!!x']) {
      expect(() => matchesPattern(p, 'a/b')).not.toThrow();
    }
  });
});
