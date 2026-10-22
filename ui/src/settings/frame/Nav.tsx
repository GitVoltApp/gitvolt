import type { RefObject } from 'react';
import { usePageModified } from '../bindings';
import { scopeLabel, type SettingsCtx, type SettingsPageDef, type SettingsScope } from '../registry';
import { useSettingsUi } from '../schema';

interface NavProps {
  pages: readonly SettingsPageDef[];
  current: string;
  /** The page whose item takes Tab (roving tabindex): the one last focused, else the shown one. */
  roving: string;
  /** Per page, its matches while a query is active; null without one. */
  counts: ReadonlyMap<string, number> | null;
  ctx: SettingsCtx;
  profile: { name: string; color: string };
  onFocusItem(id: string): void;
  navRef: RefObject<HTMLElement | null>;
}

const SCOPES: SettingsScope[] = ['app', 'profile', 'repo'];

/** Settings' nav: the pages in three groups (GitVolt, the profile, the repository). Its keys
 * (arrows, Home/End, typing to search) are the overlay's, which owns every key while it's open. */
export function SettingsNav({ pages, current, roving, counts, ctx, profile, onFocusItem, navRef }: NavProps) {
  return (
    <nav ref={navRef} className="settings-nav" aria-label="Settings pages">
      {SCOPES.map((scope) => {
        const list = pages.filter((p) => p.scope === scope);
        if (!list.length) return null;
        return (
          <div key={scope} className="settings-nav-group">
            <div className="settings-nav-title">
              {scope === 'profile' && <span className="settings-nav-profile-dot" aria-hidden style={{ background: profile.color }} />}
              {scopeLabel(scope, profile.name, ctx.repoName)}
            </div>
            {list.map((p) => (
              <NavItem key={p.id} page={p} ctx={ctx} current={p.id === current} tabbable={p.id === roving} count={counts ? counts.get(p.id) ?? 0 : null} onFocus={() => onFocusItem(p.id)} />
            ))}
          </div>
        );
      })}
    </nav>
  );
}

function NavItem({ page, ctx, current, tabbable, count, onFocus }: { page: SettingsPageDef; ctx: SettingsCtx; current: boolean; tabbable: boolean; count: number | null; onFocus(): void }) {
  const modified = usePageModified(page.id, ctx);
  const Icon = page.icon;
  return (
    <button
      type="button"
      className="settings-nav-item"
      data-page={page.id}
      data-dim={count === 0 || undefined}
      aria-current={current ? 'page' : undefined}
      tabIndex={tabbable ? 0 : -1}
      onFocus={onFocus}
      onClick={() => useSettingsUi.getState().setPage(page.id)}
    >
      <Icon size={14} aria-hidden />
      <span className="settings-nav-label">{page.label}</span>
      {count ? <span className="settings-nav-count">{count}</span> : count === null && modified && <span className="settings-nav-dot" aria-hidden />}
    </button>
  );
}
