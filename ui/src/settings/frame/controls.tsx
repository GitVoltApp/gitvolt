import type { ComponentType } from 'react';
import { Select } from '../../ui/Select';
import { Switch } from '../../ui/Switch';
import { useSettingValue } from '../bindings';
import { settingDef, type SettingsCtx } from '../registry';

type Control = ComponentType<{ ctx: SettingsCtx }>;

/** A def's on/off switch, over its binding. The row shows the label; the switch keeps it as its name. */
export function switchControl(id: string): Control {
  return function SettingSwitch({ ctx }) {
    const def = settingDef(id)!;
    const [on, set] = useSettingValue<boolean>(def, ctx);
    return <Switch id={`input-${id}`} checked={on} onChange={set} label={def.label} />;
  };
}

/** A def's dropdown (the app's own menu), over its binding; the def takes `labelFor: 'group'`. */
export function selectControl<T extends string | number>(id: string, options: ReadonlyArray<readonly [T, string]> | ((ctx: SettingsCtx, value: T) => ReadonlyArray<readonly [T, string]>)): Control {
  return function SettingSelect({ ctx }) {
    const def = settingDef(id)!;
    const [value, set] = useSettingValue<T>(def, ctx);
    return <Select<T> id={`input-${id}`} aria-labelledby={`label-${id}`} value={value} onChange={set} options={typeof options === 'function' ? options(ctx, value) : options} />;
  };
}
