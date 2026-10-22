import { RotateCcw } from 'lucide-react';
import { useRef, type ReactNode } from 'react';
import { HoverTooltip } from '../../ui/HoverTooltip';
import { resetSetting, useModified } from '../bindings';
import { settingDef, useSettingsRegistry, type SettingDef, type SettingsCtx } from '../registry';
import { useSettingsUi } from '../schema';
import type { Highlight } from '../search';

/** Focuses a setting row's control: its `input-<id>` (or checked radio), else its first button or field. */
export function focusSettingControl(row: Element): void {
  const control = row.querySelector('.setting-control') ?? row;
  (control.querySelector<HTMLElement>('input:checked, [id^="input-"]') ?? control.querySelector<HTMLElement>('button, input, select'))?.focus({ preventScroll: true });
}

/** `text` with its highlighted ranges in `<mark>`s. */
export function Marked({ text, ranges }: { text: string; ranges?: readonly Highlight[] }) {
  if (!ranges?.length) return <>{text}</>;
  const parts: ReactNode[] = [];
  let at = 0;
  for (const r of ranges) {
    if (r.start > at) parts.push(text.slice(at, r.start));
    parts.push(<mark key={r.start}>{text.slice(r.start, r.end)}</mark>);
    at = r.end;
  }
  if (at < text.length) parts.push(text.slice(at));
  return <>{parts}</>;
}

interface RowProps {
  id: string;
  ctx: SettingsCtx;
  children?: ReactNode;
  highlights?: { label: Highlight[]; description: Highlight[] };
}

/**
 * A setting's row: its label, the one-line description under it (always shown, no hover help), and
 * its control on the right (`children`, else the def's `Control`, else "Open page"). A value that
 * differs from its default shows a ↺ that restores the default, its only mark (a dot read as
 * unread); the ↺ keeps its space when hidden, so nothing moves as it comes and goes.
 */
export function SettingRow(props: RowProps) {
  useSettingsRegistry();
  const def = settingDef(props.id);
  return def ? <DefRow def={def} {...props} /> : null;
}

function DefRow({ def, ctx, children, highlights }: RowProps & { def: SettingDef }) {
  const modified = useModified(def, ctx);
  const rowRef = useRef<HTMLDivElement>(null);
  const { id, Control } = def;
  const text = <Marked text={def.label} ranges={highlights?.label} />;
  // Without a control of its own, the label names nothing: "Open page" is described by it.
  const label = (def.labelFor ?? 'input') === 'group' || (children === undefined && !Control)
    ? <span className="setting-label" id={`label-${id}`}>{text}</span>
    : <label className="setting-label" id={`label-${id}`} htmlFor={`input-${id}`}>{text}</label>;
  const restore = () => {
    resetSetting(def, ctx);
    if (rowRef.current) focusSettingControl(rowRef.current);
  };
  return (
    <div ref={rowRef} className="setting-row" data-setting-id={id} id={`setting-${id}`} data-modified={modified || undefined}>
      <div className="setting-text">
        <div className="setting-head">
          {modified && <span className="visually-hidden">Changed</span>}
          {label}
          <HoverTooltip content="Restore the default">
            <button type="button" className="icon-button setting-reset" aria-label="Restore the default" style={{ visibility: modified ? 'visible' : 'hidden' }} onClick={restore}>
              <RotateCcw size={12} aria-hidden />
            </button>
          </HoverTooltip>
        </div>
        {def.description && <p className="setting-desc"><Marked text={def.description} ranges={highlights?.description} /></p>}
      </div>
      <div className="setting-control">
        {children ?? (Control ? <Control ctx={ctx} /> : (
          <button type="button" aria-describedby={`label-${id}`} onClick={() => useSettingsUi.getState().setPage(def.page)}>Open page</button>
        ))}
      </div>
    </div>
  );
}
