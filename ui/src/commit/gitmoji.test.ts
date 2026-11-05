import { describe, expect, it } from 'vitest';
import { expandGitmoji, findGitmojiTrigger, loadGitmojis, rankGitmojis, type Gitmoji } from './gitmoji';

const LIST: Gitmoji[] = [
  { emoji: '🎨', code: 'art', description: 'Improve structure / format of the code.' },
  { emoji: '🐛', code: 'bug', description: 'Fix a bug.' },
  { emoji: '🏗️', code: 'building_construction', description: 'Make architectural changes.' },
  { emoji: '✅', code: 'white_check_mark', description: 'Add, update, or pass tests.' },
];

describe('gitmoji in the commit summary', () => {
  it('triggers on a colon at the start or after whitespace or a bracket, word characters up to the caret', () => {
    expect(findGitmojiTrigger(':', 1)).toEqual({ start: 0, query: '' });
    expect(findGitmojiTrigger(':Bu', 3)).toEqual({ start: 0, query: 'bu' });
    expect(findGitmojiTrigger(':bug fix', 4)).toEqual({ start: 0, query: 'bug' });
    expect(findGitmojiTrigger('fix :bu', 7)).toEqual({ start: 4, query: 'bu' });
    expect(findGitmojiTrigger('one\n:', 5)).toEqual({ start: 4, query: '' });
    expect(findGitmojiTrigger('(:ar', 4)).toEqual({ start: 1, query: 'ar' });
    expect(findGitmojiTrigger(':bug fix', 8)).toBeNull();
    expect(findGitmojiTrigger(':bug:', 5)).toBeNull();
    for (const t of ['http://x', 'at 12:30', 'Note:']) expect(findGitmojiTrigger(t, t.length)).toBeNull();
    expect(findGitmojiTrigger('', 0)).toBeNull();
  });

  it('keeps the codes starting with the query, in gitmoji’s order', () => {
    expect(rankGitmojis(LIST, '').map((g) => g.code)).toEqual(['art', 'bug', 'building_construction', 'white_check_mark']);
    expect(rankGitmojis(LIST, 'bu').map((g) => g.code)).toEqual(['bug', 'building_construction']);
    expect(rankGitmojis(LIST, 'check')).toEqual([]);
  });

  it('turns a whole known :code: just typed into its emoji, the caret after it', () => {
    expect(expandGitmoji(':bug:', 5, LIST)).toEqual({ text: '🐛', caret: 2, start: 0, end: 5, emoji: '🐛' });
    expect(expandGitmoji(':BUG: the thing', 5, LIST)).toEqual({ text: '🐛 the thing', caret: 2, start: 0, end: 5, emoji: '🐛' });
    expect(expandGitmoji('fix :white_check_mark:', 22, LIST)).toEqual({ text: 'fix ✅', caret: 5, start: 4, end: 22, emoji: '✅' });
    expect(expandGitmoji(':nope:', 6, LIST)).toBeNull();
    expect(expandGitmoji('at 10:bug:', 10, LIST)).toBeNull();
    expect(expandGitmoji(':bug: x', 7, LIST)).toBeNull();
  });

  it('loads the gitmoji list with its codes unwrapped', async () => {
    const list = await loadGitmojis();
    expect(list.find((g) => g.code === 'white_check_mark')?.emoji).toBe('✅');
    expect(list.every((g) => /^[a-z0-9_]+$/.test(g.code))).toBe(true);
  });
});
