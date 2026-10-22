import { useId, type ReactNode } from 'react';
import { allSettings, pageSummary, useSettingsRegistry, type SettingsCtx, type SettingsPageDef } from '../registry';
import { SettingRow } from './SettingRow';

/** A titled card of setting rows. */
export function SettingsGroup({ title, children }: { title: string; children: ReactNode }) {
  const id = useId();
  return (
    <div className="settings-group" role="group" aria-labelledby={id}>
      <h3 id={id}>{title}</h3>
      <div className="settings-card">{children}</div>
    </div>
  );
}

/**
 * A page's settings as rows, in titled groups: groups in the order their first setting registered,
 * rows in registration order. `render` lets a page's own `Body` change one group's content (add a
 * row after the settings, or show its own section in their place).
 */
export function PageGroups({ pageId, ctx, render }: { pageId: string; ctx: SettingsCtx; render?: (group: string, rows: ReactNode) => ReactNode }) {
  useSettingsRegistry();
  const groups = new Map<string, string[]>();
  for (const d of allSettings()) {
    if (d.page !== pageId) continue;
    groups.set(d.group, [...(groups.get(d.group) ?? []), d.id]);
  }
  return (
    <>
      {[...groups].map(([group, ids]) => {
        const rows = ids.map((id) => <SettingRow key={id} id={id} ctx={ctx} />);
        return <SettingsGroup key={group} title={group}>{render ? render(group, rows) : rows}</SettingsGroup>;
      })}
    </>
  );
}

/** A page: its title, the line saying what it covers, then its `Body` or its settings in groups. */
export function SettingsPageView({ page, ctx }: { page: SettingsPageDef; ctx: SettingsCtx }) {
  const titleId = useId();
  const { Body } = page;
  return (
    <section className="settings-page" aria-labelledby={titleId} data-page={page.id}>
      <header className="settings-page-head">
        <h2 id={titleId}>{page.label}</h2>
        <p className="settings-summary">{pageSummary(page, ctx)}</p>
      </header>
      {Body ? <Body ctx={ctx} /> : <PageGroups pageId={page.id} ctx={ctx} />}
    </section>
  );
}
