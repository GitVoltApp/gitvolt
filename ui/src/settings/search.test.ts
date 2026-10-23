import { FolderGit2, RefreshCw } from 'lucide-react';
import { describe, expect, it } from 'vitest';
import type { SettingDef, SettingsPageDef } from './registry';
import { searchSettings } from './search';

const pages: SettingsPageDef[] = [
  { id: 'fetch', label: 'Fetch and push', scope: 'app', order: 60, icon: RefreshCw, summary: '' },
  { id: 'graph', label: 'Graph', scope: 'app', order: 30, icon: FolderGit2, summary: '' },
];
const prune: SettingDef = { id: 'prune', label: 'Prune deleted remote branches on fetch', page: 'fetch', group: 'Fetch', keywords: 'prune --prune remote tracking' };
const defs: SettingDef[] = [
  prune,
  { id: 'gravatar', label: 'Load avatars from Gravatar', page: 'graph', group: 'Avatars', keywords: 'avatar privacy images', description: 'Looked up by a hash of the email.' },
  { id: 'forgeAvatars', label: 'From your forge accounts', page: 'graph', group: 'Avatars', keywords: 'gitlab github forge', description: 'So colleagues without an avatar elsewhere get theirs.' },
];

describe('searchSettings', () => {
  it('matches labels, descriptions and hidden keywords, every word', () => {
    const r = searchSettings('avatar', defs, pages);
    expect(r.map((p) => p.page.id)).toEqual(['graph']);
    expect(r[0].matches.map((m) => m.def.id)).toEqual(['gravatar', 'forgeAvatars']);
    expect(searchSettings('gravatar privacy', defs, pages)[0].matches.map((m) => m.def.id)).toEqual(['gravatar']);
    expect(searchSettings('prune avatar', defs, pages)).toEqual([]);
  });

  it('lists pages in nav order', () => {
    expect(searchSettings('o', defs, pages).map((p) => p.page.id)).toEqual(['graph', 'fetch']);
  });

  it('highlights each word in the label and description', () => {
    const [m] = searchSettings('avatar', defs, pages)[0].matches;
    expect(m.label).toEqual([{ start: 5, end: 11 }, { start: 20, end: 26 }]);
    expect(searchSettings('HASH email', defs, pages)[0].matches[0].description).toEqual([{ start: 15, end: 19 }, { start: 27, end: 32 }]);
  });

  it('highlights the original text under an accent', () => {
    const [m] = searchSettings('elague', [{ ...prune, label: 'Élaguer les branches' }], pages)[0].matches;
    expect(m.label).toEqual([{ start: 0, end: 6 }]);
  });

  it('is safe with regex characters, accents and blank queries', () => {
    expect(searchSettings('.*(', defs, pages)).toEqual([]);
    expect(searchSettings('   ', defs, pages)).toEqual([]);
    expect(searchSettings('', defs, pages)).toEqual([]);
    expect(searchSettings('élague', [{ ...prune, keywords: 'elague' }], pages)).toHaveLength(1);
  });
});
