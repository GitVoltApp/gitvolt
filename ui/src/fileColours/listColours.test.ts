import { describe, expect, it } from 'vitest';
import { listColoursOf } from './listColours';
import type { FileScope } from './store';

const scope = (id: string, patterns: string[], startCollapsed = false): FileScope => ({ id, name: id, tint: 'green', patterns, startCollapsed });

describe('listColoursOf', () => {
  it("a file takes its first scope's tint", () => {
    const c = listColoursOf([scope('a', ['src/**']), scope('b', ['**/*.ts'])], ['#111111', '#222222']);
    expect(c.fileTint('src/x.ts')).toBe('#111111');
    expect(c.fileTint('lib/x.ts')).toBe('#222222');
  });

  it('a folder takes a tint only when covered', () => {
    const c = listColoursOf([scope('a', ['tests/**', '*.md'])], ['#111111']);
    expect(c.folderTint('tests')).toBe('#111111');
    expect(c.folderTint('src')).toBeNull();
  });

  it('a path in no scope has none', () => {
    const c = listColoursOf([scope('a', ['tests/**'])], ['#111111']);
    expect(c.fileTint('src/a.ts')).toBeNull();
    expect(c.folderTint('src')).toBeNull();
  });

  it('only the outermost covered folder of a start-collapsed scope starts collapsed', () => {
    const c = listColoursOf([scope('gen', ['game/src/generated/**'], true), scope('docs', ['docs/**'])], ['#111111', '#222222']);
    expect(c.startsCollapsed('game/src/generated', 'game')).toBe(true);
    expect(c.startsCollapsed('game/src/generated/proto', 'game/src/generated')).toBe(false);
    expect(c.startsCollapsed('docs', null)).toBe(false);
  });

  it('a merged folder chain is outermost when the row above it is outside the scope', () => {
    const c = listColoursOf([scope('gen', ['admin/dist'], true)], ['#111111']);
    // `admin/dist/assets` is one row when dist's only child is assets: admin/dist is no row of its own.
    expect(c.startsCollapsed('admin/dist/assets', null)).toBe(true);
    expect(c.startsCollapsed('admin/dist/assets', 'admin')).toBe(true);
    expect(c.startsCollapsed('admin/dist/assets/img', 'admin/dist/assets')).toBe(false);
  });
});
