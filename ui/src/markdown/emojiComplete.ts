/** Emoji autocomplete, in the Markdown fields and the commit message alike: a `:` offers the
 * gitmojis matching what follows it (with their meaning), then the other emojis. */
import { rankGitmojis, type Gitmoji } from './gitmoji';

export interface EmojiEntry { emoji: string; names: string[] }
/** `description`: a gitmoji's meaning. */
export interface EmojiHit { emoji: string; name: string; description?: string }
export interface Trigger { /** Index of the `:`. */ start: number; query: string }

export const MAX_HITS = 8;

/** The emoji popup lists this many (it scrolls): all the gitmojis (74 today) for a bare `:`. */
export const EMOJI_MAX = 100;

/** gemoji has no popularity order, so the everyday ones are named here and sort first. */
const COMMON = ['+1', 'thumbsup', '-1', 'thumbsdown', 'smile', 'tada', 'heart', 'eyes', 'rocket', 'white_check_mark', 'warning', 'bug', 'fire', 'pray', 'joy', 'thinking', 'clap', 'sparkles', 'x', 'memo'];
const COMMON_RANK = new Map(COMMON.map((n, i) => [n, i]));

const inFence = (before: string): boolean => {
  let open = false;
  for (const line of before.split('\n').slice(0, -1)) if (/^\s{0,3}(```|~~~)/.test(line)) open = !open;
  return open;
};

/** Whether the character at `start` is inside fenced code or an inline code span. */
export function inCode(before: string, start: number): boolean {
  if (inFence(before)) return true;
  const lineStart = before.lastIndexOf('\n') + 1;
  return (before.slice(lineStart, start).match(/`/g)?.length ?? 0) % 2 === 1;
}

/** The shortcode being typed just before `caret`, or null (no popup): the colon starts the text
 * or follows whitespace or an opening bracket (so not `http://`, `12:30` or `a:b`), the caret is
 * after it or after word characters following it, and not in fenced or inline code. */
export function findTrigger(text: string, caret: number): Trigger | null {
  const before = text.slice(0, caret);
  const m = /(^|[\s([{]):([a-z0-9_+-]*)$/i.exec(before);
  if (!m) return null;
  const start = m.index + m[1].length;
  if (inCode(before, start)) return null;
  return { start, query: m[2].toLowerCase() };
}

/** Up to `limit` matches: names (and aliases) starting with the query, then those containing
 * it, each group with the common ones first and otherwise in gemoji's order. */
export function rankEmoji(list: readonly EmojiEntry[], query: string, limit = MAX_HITS): EmojiHit[] {
  const q = query.toLowerCase();
  type Row = EmojiHit & { c: number; i: number };
  const prefix: Row[] = [];
  const inner: Row[] = [];
  list.forEach((e, i) => {
    let best: { name: string; pre: boolean } | null = null;
    for (const name of e.names) {
      const at = name.indexOf(q);
      if (at < 0) continue;
      if (at === 0) { best = { name, pre: true }; break; }
      best ??= { name, pre: false };
    }
    if (!best) return;
    const c = Math.min(...e.names.map((n) => COMMON_RANK.get(n) ?? 99));
    (best.pre ? prefix : inner).push({ emoji: e.emoji, name: best.name, c, i });
  });
  const by = (a: Row, b: Row) => a.c - b.c || a.i - b.i;
  return [...prefix.sort(by), ...inner.sort(by)].slice(0, limit).map(({ emoji, name }) => ({ emoji, name }));
}

/** The same emoji, with or without the variation selector (gitmoji's 🏗️, gemoji's 🏗). */
const bare = (emoji: string) => emoji.replace(/\uFE0F/g, '');

/** Up to `limit` hits: the gitmojis whose code starts with the query (with their meaning), then
 * `rankEmoji`'s, less the emojis already listed. Either list may still be loading (null). */
export function rankHits(gitmojis: readonly Gitmoji[] | null, emojis: readonly EmojiEntry[] | null, query: string, limit = EMOJI_MAX): EmojiHit[] {
  const q = query.toLowerCase();
  const first = (gitmojis ? rankGitmojis(gitmojis, q) : []).map((g) => ({ emoji: g.emoji, name: g.code, description: g.description }));
  const seen = new Set(first.map((h) => bare(h.emoji)));
  const rest = (emojis ? rankEmoji(emojis, q, limit) : []).filter((h) => !seen.has(bare(h.emoji)));
  return [...first, ...rest].slice(0, limit);
}

/** `text` with the typed trigger and query replaced by `token`, and the caret after it. */
export function insertToken(text: string, caret: number, trigger: Trigger, token: string): { text: string; caret: number } {
  return { text: text.slice(0, trigger.start) + token + text.slice(caret), caret: trigger.start + token.length };
}

/** `insertToken` for `:name: `. */
export const insertShortcode = (text: string, caret: number, trigger: Trigger, name: string) => insertToken(text, caret, trigger, `:${name}: `);

/** The list, loaded once on first need as gemoji's own chunk. */
let listing: Promise<readonly EmojiEntry[]> | null = null;
export function loadEmojiList(): Promise<readonly EmojiEntry[]> {
  listing ??= import('gemoji').then((m) => m.gemoji, (e: unknown) => { listing = null; throw e; });
  return listing;
}
