import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { THEMES } from '../theme/themes';
import { useTheme } from '../theme/store';
import { currentGround, useScopeTints } from './useTints';

const alpha = (c: string | null | undefined) => Number(c?.match(/([\d.]+)\)$/)?.[1]);

beforeEach(() => act(() => useTheme.getState().set('default-dark', {})));

describe('useTints', () => {
  it('currentGround reads the applied theme', () => {
    act(() => useTheme.getState().set('light', {}));
    const g = currentGround();
    expect(g.kind).toBe('light');
    expect(g.bg).toBe(THEMES.light.colors['panel-bg0']);
  });

  it('useScopeTints recomputes on a theme switch', () => {
    const scopes = [{ tint: 'green' }];
    const { result } = renderHook(() => useScopeTints(scopes));
    expect(alpha(result.current[0])).toBe(0.1);
    act(() => useTheme.getState().set('light', {}));
    expect(alpha(result.current[0])).toBe(0.14);
  });

  it('useScopeTints keeps its array while nothing changes', () => {
    const scopes = [{ tint: 'green' }];
    const { result, rerender } = renderHook(() => useScopeTints(scopes));
    const first = result.current;
    rerender();
    expect(result.current).toBe(first);
  });
});
