import type { ReactNode } from 'react';
import { useSettingsCtx } from './frame/ctx';
import { SettingRow } from './frame/SettingRow';
// The defs this wrapper's one user (theme/AppearanceSection) names, wherever it's rendered.
import './pages/appearance';

/** A setting's row, for sections other modules own (theme/AppearanceSection) until the theme
 * editor replaces them: `SettingRow` for the shown tab. `group`: the control is labelled by
 * `label-<id>` rather than a `<label for>`. */
export function Row({ id, children, group = false }: { id: string; children: ReactNode; group?: boolean }) {
  const ctx = useSettingsCtx();
  return <SettingRow id={id} ctx={ctx} labelFor={group ? 'group' : 'input'}>{children}</SettingRow>;
}
