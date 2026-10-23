import { useMemo } from 'react';
import { useTheme } from '../theme/store';
import { rowTint, type TintGround } from './tint';

/**
 * The applied theme as a ground, read from the custom properties `applyTheme` writes on <html>.
 * This is the one place that knows how the theme is applied, so custom themes work unchanged.
 */
export function currentGround(): TintGround {
  const root = document.documentElement;
  const read = (name: string) => root.style.getPropertyValue(name).trim();
  return {
    kind: root.style.colorScheme === 'light' ? 'light' : 'dark',
    bg: read('--panel-bg0'),
    texts: [read('--text-normal'), read('--text-secondary')].filter(Boolean),
  };
}

/** One fill per scope (null for an unusable tint); recomputed only when the scopes or the theme change. */
export function useScopeTints(scopes: readonly { tint: string }[]): readonly (string | null)[] {
  const version = useTheme((s) => s.version);
  return useMemo(() => {
    const ground = currentGround();
    return scopes.map((s) => rowTint(s.tint, ground));
    // `version` stands for the applied theme: currentGround reads it from the DOM.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scopes, version]);
}
