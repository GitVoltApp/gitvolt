import { useRef, type ComponentType } from 'react';
import { useKeys } from '../../ui/keyRouter';
import { Select } from '../../ui/Select';
import { Switch } from '../../ui/Switch';
import { useSettingValue } from '../bindings';
import { settingDef, type SettingsCtx } from '../registry';

type Control = ComponentType<{ ctx: SettingsCtx }>;

const NEXT: Record<string, 1 | -1> = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 };

interface ChoiceGroupProps<T extends string> {
  /** The setting's id: the group is labelled by its row's `label-<id>`. */
  id: string;
  value: T;
  options: ReadonlyArray<readonly [T, string]>;
  onChange(value: T): void;
  disabled?: boolean;
}

/**
 * A setting's choice of a few, as one row of joined buttons (the toolbars' `.segmented`), the
 * chosen one highlighted. A radio group: Tab stops on the chosen button, the arrows (wrapping),
 * Home and End move and choose, Space or Enter chooses. Keys go through the key router: in
 * Settings the dialog's claim stops them before React's.
 */
export function ChoiceGroup<T extends string>({ id, value, options, onChange, disabled }: ChoiceGroupProps<T>) {
  const group = useRef<HTMLDivElement>(null);
  const stop = options.some(([v]) => v === value) ? value : options[0]?.[0];
  useKeys('menu', (e) => {
    const button = e.target instanceof Element ? e.target.closest<HTMLButtonElement>('[role="radio"]') : null;
    if (!button || !group.current?.contains(button) || button.disabled || e.altKey || e.ctrlKey || e.metaKey) return;
    const all = [...group.current.querySelectorAll<HTMLButtonElement>('[role="radio"]')];
    const i = all.indexOf(button);
    const to = e.key in NEXT ? (i + NEXT[e.key]! + all.length) % all.length : e.key === 'Home' ? 0 : e.key === 'End' ? all.length - 1 : e.key === ' ' || e.key === 'Enter' ? i : null;
    if (to === null) return;
    e.preventDefault();
    if (!e.repeat || to !== i) {
      all[to]!.focus();
      onChange(options[to]![0]);
    }
    return 'handled';
  });
  return (
    <div ref={group} role="radiogroup" aria-labelledby={`label-${id}`} aria-disabled={disabled || undefined} className="segmented choice-group">
      {options.map(([v, label]) => (
        <button key={v} type="button" role="radio" aria-checked={v === value} tabIndex={v === stop ? 0 : -1} disabled={disabled} onClick={() => onChange(v)}>{label}</button>
      ))}
    </div>
  );
}

/** A def's [`ChoiceGroup`], over its binding; the def takes `labelFor: 'group'`. */
export function choiceControl<T extends string>(id: string, options: ReadonlyArray<readonly [T, string]>): Control {
  return function SettingChoice({ ctx }) {
    const def = settingDef(id)!;
    const [value, set] = useSettingValue<T>(def, ctx);
    return <ChoiceGroup<T> id={id} value={value} options={options} onChange={set} />;
  };
}

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
