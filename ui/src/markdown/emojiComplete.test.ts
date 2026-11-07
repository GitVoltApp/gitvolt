import { describe, expect, it } from 'vitest';
import { findTrigger, insertShortcode, rankEmoji, rankHits, shortcodes, typedShortcode, type EmojiEntry } from './emojiComplete';
import type { Gitmoji } from './gitmoji';

const t = (s: string) => findTrigger(s, s.length);
const L: EmojiEntry[] = [
  { emoji: 'A', names: ['smiley'] },
  { emoji: 'B', names: ['grin_smile'] },
  { emoji: 'C', names: ['smile'] },
  { emoji: 'D', names: ['+1', 'thumbsup'] },
  { emoji: 'E', names: ['thumbsdown', '-1'] },
];

describe('findTrigger', () => {
  it('fires at the start, after a space or newline, and after an opening bracket', () => {
    expect(t(':sm')).toEqual({ start: 0, query: 'sm' });
    expect(t('hi :+1')).toEqual({ start: 3, query: '+1' });
    expect(t('a\n:thu')).toEqual({ start: 2, query: 'thu' });
    expect(t('(:sm')).toEqual({ start: 1, query: 'sm' });
  });
  it('opens on the colon itself', () => {
    expect(t(':')).toEqual({ start: 0, query: '' });
    expect(t('fix :B')).toEqual({ start: 4, query: 'b' });
    expect(t('one\n:')).toEqual({ start: 4, query: '' });
    expect(t('fix : it')).toBeNull();
    expect(t('Note:')).toBeNull();
    expect(t('')).toBeNull();
  });
  it('stays quiet in URLs, times and words', () => {
    expect(t('see http://example')).toBeNull();
    expect(t('at 12:30')).toBeNull();
    expect(t('a:bc')).toBeNull();
  });
  it('stays quiet after a finished shortcode', () => { expect(t(':smile:')).toBeNull(); });
  it('stays quiet in inline and fenced code', () => {
    expect(t('run `x :sm')).toBeNull();
    expect(t('```\n:sm')).toBeNull();
    expect(t('```\ncode\n```\n:sm')).not.toBeNull();
    expect(t('`a` :sm')).not.toBeNull();
  });
  it('uses the caret, not the end', () => { expect(findTrigger(':sm tail', 3)).toEqual({ start: 0, query: 'sm' }); });
});

describe('rankEmoji', () => {
  it('puts prefix matches before substring matches, common names first', () => {
    expect(rankEmoji(L, 'smi').map((h) => h.name)).toEqual(['smile', 'smiley', 'grin_smile']);
  });
  it('matches aliases and reports the matching name', () => {
    expect(rankEmoji(L, 'thu').map((h) => h.name)).toEqual(['thumbsup', 'thumbsdown']);
    expect(rankEmoji(L, '+1')[0]).toEqual({ emoji: 'D', name: '+1' });
  });
  it('caps the list', () => { expect(rankEmoji(L, 'sm', 2)).toHaveLength(2); });
  it('finds nothing for nonsense', () => { expect(rankEmoji(L, 'zzz')).toEqual([]); });
});

describe('insertShortcode', () => {
  it('replaces the typed query with :name: and a space', () => {
    expect(insertShortcode('hi :thu there', 7, { start: 3, query: 'thu' }, 'thumbsup')).toEqual({ text: 'hi :thumbsup:  there', caret: 14 });
  });
});

const G: Gitmoji[] = [
  { emoji: '🐛', code: 'bug', description: 'Fix a bug.' },
  { emoji: '🏗️', code: 'building_construction', description: 'Make architectural changes.' },
  { emoji: '✨', code: 'sparkles', description: 'Introduce new features.' },
];
const E: EmojiEntry[] = [
  { emoji: '🚌', names: ['bus'] },
  { emoji: '🐛', names: ['bug'] },
  { emoji: '🏗', names: ['building_construction'] },
  { emoji: '😄', names: ['smile'] },
  { emoji: '🦋', names: ['butterfly'] },
];

describe('rankHits', () => {
  it('lists the matching gitmojis first, with their meaning, then the other emojis without repeats', () => {
    expect(rankHits(G, E, 'bu')).toEqual([
      { emoji: '🐛', name: 'bug', description: 'Fix a bug.' },
      { emoji: '🏗️', name: 'building_construction', description: 'Make architectural changes.' },
      { emoji: '🚌', name: 'bus' },
      { emoji: '🦋', name: 'butterfly' },
    ]);
  });
  it('a bare colon: every gitmoji first', () => {
    expect(rankHits(G, E, '').slice(0, 3).map((h) => h.name)).toEqual(['bug', 'building_construction', 'sparkles']);
  });
  it('either list may still be loading, and the whole is capped', () => {
    expect(rankHits(null, E, 'bu').map((h) => h.name)).toEqual(['bug', 'bus', 'building_construction', 'butterfly']);
    expect(rankHits(G, null, 'sp').map((h) => h.name)).toEqual(['sparkles']);
    expect(rankHits(G, E, '', 2)).toHaveLength(2);
  });
});

describe('typedShortcode', () => {
  const names = shortcodes(G, E);
  it('finds a whole known :code: just typed, a gitmoji or any emoji', () => {
    expect(typedShortcode(':bug:', 5, names)).toEqual({ start: 0, end: 5, emoji: '🐛' });
    expect(typedShortcode('fix :SMILE:', 11, names)).toEqual({ start: 4, end: 11, emoji: '😄' });
    expect(typedShortcode('x :building_construction:', 25, names)?.emoji).toBe('🏗️');
  });
  it('not an unknown code, one inside a word, or one the caret has left', () => {
    expect(typedShortcode(':nope:', 6, names)).toBeNull();
    expect(typedShortcode('at 10:bug:', 10, names)).toBeNull();
    expect(typedShortcode(':bug: x', 7, names)).toBeNull();
  });
});
