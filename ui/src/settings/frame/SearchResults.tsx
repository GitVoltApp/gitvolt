import { useId } from 'react';
import { scopeLabel, type SettingsCtx } from '../registry';
import { useSettingsUi } from '../schema';
import type { PageMatches, SettingMatch } from '../search';
import { SettingRow } from './SettingRow';

/** Consecutive matches of one group, so each card's breadcrumb names it. */
function byGroup(matches: readonly SettingMatch[]): Array<[string, SettingMatch[]]> {
  const out: Array<[string, SettingMatch[]]> = [];
  for (const m of matches) {
    const last = out[out.length - 1];
    if (last && last[0] === m.def.group) last[1].push(m);
    else out.push([m.def.group, [m]]);
  }
  return out;
}

/**
 * Settings' search: every match in place, as a live row with its words highlighted, in one card
 * per page and group headed by where it lives (`GitVolt › Graph › Avatars`) and "Open page".
 */
export function SearchResults({ query, results, ctx, profileName }: { query: string; results: readonly PageMatches[]; ctx: SettingsCtx; profileName: string }) {
  const titleId = useId();
  const q = query.trim();
  const n = results.reduce((sum, p) => sum + p.matches.length, 0);
  return (
    <section className="settings-page settings-results" aria-labelledby={titleId}>
      <header className="settings-page-head">
        <h2 id={titleId}>{n === 0 ? `No settings match "${q}"` : n === 1 ? `1 setting matches "${q}"` : `${n} settings match "${q}"`}</h2>
        {n > 0 && <p className="settings-summary">Change them here, or open their page.</p>}
      </header>
      {results.flatMap(({ page, matches }) => byGroup(matches).map(([group, ms]) => (
        <div key={`${page.id}:${group}`} className="settings-group" role="group" aria-label={`${page.label} › ${group}`}>
          <div className="settings-crumb">
            <span>{scopeLabel(page.scope, profileName, ctx.repoName)} › <strong>{page.label}</strong> › {group}</span>
            <button type="button" className="link-button" onClick={() => useSettingsUi.getState().show(ms[0].def.id)}>Open page</button>
          </div>
          <div className="settings-card">
            {ms.map((m) => <SettingRow key={m.def.id} id={m.def.id} ctx={ctx} highlights={m} />)}
          </div>
        </div>
      )))}
    </section>
  );
}
