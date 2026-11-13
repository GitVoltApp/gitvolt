/** The gitmojis (gitmoji.dev): the emoji completion lists those matching first, each with its
 * meaning, then the other emojis (`emojiComplete.ts`). */

/** `code` without its colons (`white_check_mark`), as typed. */
export interface Gitmoji { emoji: string; code: string; description: string }

/** The gitmojis whose code starts with `query`, in gitmoji's own order. */
export function rankGitmojis(list: readonly Gitmoji[], query: string): Gitmoji[] {
  return list.filter((g) => g.code.startsWith(query));
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
