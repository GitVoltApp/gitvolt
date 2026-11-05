import type { Trigger } from '../markdown/emojiComplete';

/** Gitmoji in the commit message (Settings › Repository › Commit message): a `:` offers the
 * gitmojis, narrowed to the codes starting with what follows it, as Slack's emoji picker. */

/** `code` without its colons (`white_check_mark`), as typed. */
export interface Gitmoji { emoji: string; code: string; description: string }

/** The popup lists them all (74 today) and scrolls. */
export const GITMOJI_MAX = 100;

/** A colon that starts a code: first in the text, or after whitespace or an opening bracket (so
 * not `http://`, `12:30` or `Note:`). */
const LEAD = '(^|[\\s([{])';
const TYPING = new RegExp(`${LEAD}:([a-z0-9_]*)$`, 'i');
const TYPED = new RegExp(`${LEAD}:([a-z0-9_]+):$`, 'i');

/** The code being typed just before `caret`: a colon (see `LEAD`), word characters after it. */
export function findGitmojiTrigger(text: string, caret: number): Trigger | null {
  const m = TYPING.exec(text.slice(0, caret));
  return m ? { start: m.index + m[1].length, query: m[2].toLowerCase() } : null;
}

/** The gitmojis whose code starts with `query`, in gitmoji's own order. */
export function rankGitmojis(list: readonly Gitmoji[], query: string): Gitmoji[] {
  return list.filter((g) => g.code.startsWith(query));
}

/** A whole known `:code:` just typed (it ends at `caret`) becomes its emoji, the caret after it;
 * null otherwise. */
export function expandGitmoji(text: string, caret: number, list: readonly Gitmoji[]): { text: string; caret: number; start: number; end: number; emoji: string } | null {
  const m = TYPED.exec(text.slice(0, caret));
  const hit = m && list.find((g) => g.code === m[2].toLowerCase());
  if (!m || !hit) return null;
  const start = m.index + m[1].length;
  return { text: text.slice(0, start) + hit.emoji + text.slice(caret), caret: start + hit.emoji.length, start, end: caret, emoji: hit.emoji };
}

/** The list, loaded once on first need as its own chunk. */
let listing: Promise<readonly Gitmoji[]> | null = null;
export function loadGitmojis(): Promise<readonly Gitmoji[]> {
  listing ??= import('gitmojis').then(
    (m) => m.gitmojis.map((g) => ({ emoji: g.emoji, code: g.code.replace(/^:|:$/g, ''), description: g.description })),
    (e: unknown) => { listing = null; throw e; },
  );
  return listing;
}
