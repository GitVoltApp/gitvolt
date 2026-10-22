// @vitest-environment happy-dom
import { Settings } from 'lucide-react';
import { describe, expect, it } from 'vitest';
import { allSettings, registerSettings, registerSettingsPage, settingDef, settingsPages, type SettingsPageDef } from './registry';

const page = (id: string, scope: SettingsPageDef['scope'], order: number): SettingsPageDef => ({ id, label: id, scope, order, icon: Settings, summary: '' });

describe('settings registry', () => {
  it('returns pages in nav order: app, then profile, then repo, then by order', () => {
    const offs = [page('t-repo', 'repo', 1), page('t-app2', 'app', 20), page('t-profile', 'profile', 5), page('t-app1', 'app', 10)].map(registerSettingsPage);
    expect(settingsPages().map((p) => p.id)).toEqual(['t-app1', 't-app2', 't-profile', 't-repo']);
    offs.forEach((off) => off());
    expect(settingsPages()).toEqual([]);
  });

  it('refuses a duplicate page or setting id', () => {
    const off = registerSettingsPage(page('t-dup', 'app', 1));
    expect(() => registerSettingsPage(page('t-dup', 'app', 2))).toThrow(/already registered/);
    off();
    const offDefs = registerSettings([{ id: 't-a', label: 'A', page: 't-dup', group: 'G', keywords: '' }]);
    expect(() => registerSettings([{ id: 't-a', label: 'A again', page: 't-dup', group: 'G', keywords: '' }])).toThrow(/already registered/);
    expect(() => registerSettings([{ id: 't-b', label: 'B', page: 'x', group: 'G', keywords: '' }, { id: 't-b', label: 'B', page: 'x', group: 'G', keywords: '' }])).toThrow(/already registered/);
    offDefs();
  });

  it('keeps settings in registration order; unregistering removes them and bumps the version', async () => {
    const { renderHook, act } = await import('@testing-library/react');
    const { useSettingsRegistry } = await import('./registry');
    const { result } = renderHook(() => useSettingsRegistry());
    const before = result.current;
    let off = () => {};
    act(() => { off = registerSettings([{ id: 't-2', label: '2', page: 'p', group: 'G', keywords: '' }, { id: 't-1', label: '1', page: 'p', group: 'G', keywords: '' }]); });
    expect(allSettings().map((d) => d.id)).toEqual(['t-2', 't-1']);
    expect(result.current).toBeGreaterThan(before);
    const mid = result.current;
    act(() => off());
    expect(settingDef('t-1')).toBeUndefined();
    expect(result.current).toBeGreaterThan(mid);
  });
});
