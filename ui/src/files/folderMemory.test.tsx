import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { collapsedSet, NEVER_COLLAPSED, useFolderCollapse, useFolderMemory } from './folderMemory';

describe('folder memory', () => {
  beforeEach(() => useFolderMemory.setState({ byRepo: {} }));

  it('a remembered folder overrides its default and the others follow it', () => {
    const gen = (f: string) => f === 'gen';
    expect([...collapsedSet(['gen', 'src'], undefined, gen)]).toEqual(['gen']);
    expect([...collapsedSet(['gen', 'src'], { gen: true, src: false }, gen)]).toEqual(['src']);
    expect([...collapsedSet(['gen', 'src'], { src: false }, NEVER_COLLAPSED)]).toEqual(['src']);
  });

  it('memory is per repository', () => {
    useFolderMemory.getState().remember('/a', { src: false });
    const a = renderHook(() => useFolderCollapse('/a', ['src']));
    const b = renderHook(() => useFolderCollapse('/b', ['src']));
    expect([...a.result.current[0]]).toEqual(['src']);
    expect([...b.result.current[0]]).toEqual([]);
  });

  it('the setter remembers only the folders whose state changed', () => {
    const folders = ['gen', 'src'];
    const starts = (f: string) => f === 'gen';
    const { result } = renderHook(() => useFolderCollapse('/a', folders, starts));
    expect([...result.current[0]]).toEqual(['gen']);
    act(() => result.current[1]((c) => new Set([...c, 'src'])));
    expect(useFolderMemory.getState().byRepo['/a']).toEqual({ src: false });
    expect([...result.current[0]].sort()).toEqual(['gen', 'src']);
  });
});
