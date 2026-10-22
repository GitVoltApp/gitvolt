import { byNavOrder, type SettingDef, type SettingsPageDef } from './registry';

export interface Highlight { start: number; end: number }
export interface SettingMatch { def: SettingDef; label: Highlight[]; description: Highlight[] }
export interface PageMatches { page: SettingsPageDef; matches: SettingMatch[] }

const MARKS = /[̀-ͯ]/g;
const fold = (s: string) => s.normalize('NFD').replace(MARKS, '').toLowerCase();

/** `text` folded (no accents, lower case), with each folded character's index in `text`, so a
 * match found in the folded text highlights the original. */
function folded(text: string): { text: string; at: number[] } {
  let out = '';
  const at: number[] = [];
  let i = 0;
  for (const ch of text) {
    const f = fold(ch);
    out += f;
    for (let k = 0; k < f.length; k++) at.push(i);
    i += ch.length;
  }
  return { text: out, at };
}

function highlights(text: string, words: readonly string[]): Highlight[] {
  const f = folded(text);
  const found: Highlight[] = [];
  for (const w of words) {
    for (let i = f.text.indexOf(w); i >= 0; i = f.text.indexOf(w, i + 1)) {
      const last = f.at[i + w.length - 1];
      found.push({ start: f.at[i], end: last + (text.codePointAt(last)! > 0xffff ? 2 : 1) });
    }
  }
  found.sort((a, b) => a.start - b.start);
  const merged: Highlight[] = [];
  for (const h of found) {
    const prev = merged[merged.length - 1];
    if (prev && h.start <= prev.end) prev.end = Math.max(prev.end, h.end);
    else merged.push({ ...h });
  }
  return merged;
}

/** Each whitespace-separated word must occur (case- and accent-insensitive substring) in the label,
 * description or keywords. Pages in nav order, settings in registration order. Empty query: []. */
export function searchSettings(query: string, defs: readonly SettingDef[], pages: readonly SettingsPageDef[]): PageMatches[] {
  const words = fold(query).split(/\s+/).filter(Boolean);
  if (!words.length) return [];
  const byPage = new Map<string, SettingMatch[]>();
  for (const def of defs) {
    const hay = fold(`${def.label}\n${def.description ?? ''}\n${def.keywords}`);
    if (!words.every((w) => hay.includes(w))) continue;
    const list = byPage.get(def.page) ?? [];
    list.push({ def, label: highlights(def.label, words), description: highlights(def.description ?? '', words) });
    byPage.set(def.page, list);
  }
  return [...pages].sort(byNavOrder).flatMap((page) => {
    const matches = byPage.get(page.id);
    return matches ? [{ page, matches }] : [];
  });
}
