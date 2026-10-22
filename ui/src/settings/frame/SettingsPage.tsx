import { useId, type ReactNode } from 'react';
import { allSettings, pageSummary, useSettingsRegistry, type SettingsCtx, type SettingsPageDef } from '../registry';
import { SettingRow } from './SettingRow';

/** A titled card of setting rows. `bare`: the content draws its own frame (the theme cards).
 * `hint`: a few words after the title ("drag to reorder, click to change"). */
export function SettingsGroup({ title, hint, bare = false, children }: { title: string; hint?: string; bare?: boolean; children: ReactNode }) {
  const id = useId();
  return (
    <div className="settings-group" role="group" aria-labelledby={id}>
      <h3 id={id}>{title}{hint && <span className="settings-group-hint"> · {hint}</span>}</h3>
      {bare ? children : <div className="settings-card">{children}</div>}
    </div>
  );
}

/**
 * A page's settings as rows, in titled groups: groups in the order their first setting registered,
 * rows in registration order. `render` lets a page's own `Body` change one group's content (add a
 * row after the settings, or show its own section in their place); `skip` leaves out the groups
 * it draws itself.
 */
export function PageGroups({ pageId, ctx, render, skip }: { pageId: string; ctx: SettingsCtx; render?: (group: string, rows: ReactNode) => ReactNode; skip?: ReadonlySet<string> }) {
  useSettingsRegistry();
  const groups = new Map<string, string[]>();
  for (const d of allSettings()) {
    if (d.page !== pageId || skip?.has(d.group)) continue;
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
