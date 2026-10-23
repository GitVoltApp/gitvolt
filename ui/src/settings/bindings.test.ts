import { Settings } from 'lucide-react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../api/client', () => ({ api: { saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null) } }));

const { DEFAULT_SETTINGS, EMPTY_PROFILE, useAppState } = await import('../app/state');
const { appSetting, isModified, profileField, repoField, resetAppSettings } = await import('./bindings');
const { registerSettings, registerSettingsPage } = await import('./registry');

const ctx = { repoPath: null, repoName: null, tabId: null };

describe('setting bindings', () => {
  beforeEach(() => useAppState.setState({ settings: { ...DEFAULT_SETTINGS }, profile: { ...EMPTY_PROFILE } }));

  it('an app setting is modified once it differs from its default, and sets through setSettings', () => {
    const setSettings = vi.spyOn(useAppState.getState(), 'setSettings');
    const b = appSetting('prune', true)(ctx);
    expect(isModified(b)).toBe(false);
    b.set(false);
    expect(setSettings).toHaveBeenCalledWith({ prune: false });
    expect(useAppState.getState().settings.prune).toBe(false);
    expect(isModified(b)).toBe(true);
  });

  it('a repo field needs a repository; an absent optional field is its null default', () => {
    expect(repoField('editor', null)(ctx)).toBeNull();
    const b = repoField('branchUpdate', undefined)({ ...ctx, repoPath: '/r/orbital' })!;
    expect(isModified(b)).toBe(false);
    b.set('merge');
    expect(useAppState.getState().profile.repos['/r/orbital'].branchUpdate).toBe('merge');
    expect(isModified(b)).toBe(true);
  });

  describe('resetAppSettings', () => {
    const offs: Array<() => void> = [];
    afterEach(() => offs.splice(0).forEach((off) => off()));

    it('resets the settings on app pages, running their side effect, and never a profile one', () => {
      const after = vi.fn();
      offs.push(
        registerSettingsPage({ id: 't-fetch', label: 'Fetch', scope: 'app', order: 1, icon: Settings, summary: '' }),
        registerSettingsPage({ id: 't-folders', label: 'Folders', scope: 'profile', order: 1, icon: Settings, summary: '' }),
        registerSettings([
          { id: 't-prune', label: 'Prune', page: 't-fetch', group: 'Fetch', keywords: '', binding: appSetting('prune', true, after) },
          { id: 't-reposFolder', label: 'Repos', page: 't-folders', group: 'Folders', keywords: '', binding: profileField('reposFolder', null) },
        ]),
      );
      appSetting('prune', true)(ctx).set(false);
      profileField('reposFolder', null)(ctx).set('/srv/repos');
      resetAppSettings(ctx);
      expect(useAppState.getState().settings.prune).toBe(true);
      expect(after).toHaveBeenCalledWith(true);
      expect(useAppState.getState().profile.reposFolder).toBe('/srv/repos');
    });
  });
});
