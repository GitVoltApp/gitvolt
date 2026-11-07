import { describe, expect, it } from 'vitest';
import { loadGitmojis, rankGitmojis, type Gitmoji } from './gitmoji';

const LIST: Gitmoji[] = [
  { emoji: '🎨', code: 'art', description: 'Improve structure / format of the code.' },
  { emoji: '🐛', code: 'bug', description: 'Fix a bug.' },
  { emoji: '🏗️', code: 'building_construction', description: 'Make architectural changes.' },
  { emoji: '✅', code: 'white_check_mark', description: 'Add, update, or pass tests.' },
];

describe('gitmojis', () => {
  it('keeps the codes starting with the query, in gitmoji’s order', () => {
    expect(rankGitmojis(LIST, '').map((g) => g.code)).toEqual(['art', 'bug', 'building_construction', 'white_check_mark']);
    expect(rankGitmojis(LIST, 'bu').map((g) => g.code)).toEqual(['bug', 'building_construction']);
    expect(rankGitmojis(LIST, 'check')).toEqual([]);
  });

  it('loads the gitmoji list with its codes unwrapped', async () => {
    const list = await loadGitmojis();
    expect(list.find((g) => g.code === 'white_check_mark')?.emoji).toBe('✅');
    expect(list.every((g) => /^[a-z0-9_]+$/.test(g.code))).toBe(true);
  });
});
