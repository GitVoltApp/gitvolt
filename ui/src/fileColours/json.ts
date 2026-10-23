import { MAX_SHARE_BYTES, parseShareJson, toShareJson } from '../settings/share';
import { uniqueName } from './scopes';
import { newScopeId, type FileScope } from './store';
import { TINT_PRESETS } from './tint';

const MAX_BYTES = 1_000_000;
const MAX_SCOPES = 200;
const MAX_PATTERNS = 200;

export function scopesToJson(scopes: readonly FileScope[]): string {
  return toShareJson('file-colours', 1, {
    scopes: scopes.map(({ name, tint, patterns, startCollapsed }) => ({ name, tint, patterns, startCollapsed })),
  });
}

/** null: not JSON, another kind, a newer version, no scopes array, or too big. */
export function parseScopesJson(text: string): Omit<FileScope, 'id'>[] | null {
  if (new TextEncoder().encode(text).length > MAX_BYTES) return null;
  const body = envelopeBody(text);
  const list = body?.scopes;
  if (!Array.isArray(list) || list.length > MAX_SCOPES) return null;
  const out: Omit<FileScope, 'id'>[] = [];
  for (const raw of list) {
    if (!raw || typeof raw !== 'object') continue;
    const r = raw as Record<string, unknown>;
    const name = typeof r.name === 'string' ? r.name.trim() : '';
    const patterns = (Array.isArray(r.patterns) ? r.patterns : [])
      .filter((p): p is string => typeof p === 'string')
      .map((p) => p.trim())
      .filter(Boolean)
      .slice(0, MAX_PATTERNS);
    if (!name || patterns.length === 0) continue;
    const tint = typeof r.tint === 'string' && (r.tint in TINT_PRESETS || /^#[0-9a-f]{6}$/i.test(r.tint)) ? r.tint : 'gray';
    out.push({ name, tint, patterns, startCollapsed: r.startCollapsed === true });
  }
  return out;
}

/** share.ts's envelope check; it caps texts at MAX_SHARE_BYTES, and a scope list may run to 1 MB, so
 * larger texts get the same checks here. */
function envelopeBody(text: string): Record<string, unknown> | null {
  const shared = parseShareJson(text, 'file-colours', 1);
  if (shared.ok) return shared.body;
  if (new TextEncoder().encode(text).length <= MAX_SHARE_BYTES) return null;
  const raw = text.trim().replace(/^```(?:json)?[ \t]*\r?\n?/i, '').replace(/\r?\n?```$/, '').trim();
  try {
    const { gitvolt, version, ...body } = JSON.parse(raw) as Record<string, unknown>;
    return gitvolt === 'file-colours' && version === 1 ? body : null;
  } catch {
    return null;
  }
}

/** The incoming scopes after the existing ones: fresh ids, clashing names renamed. */
export function appendScopes(existing: readonly FileScope[], incoming: readonly Omit<FileScope, 'id'>[]): FileScope[] {
  const names = existing.map((s) => s.name);
  const added = incoming.map((s) => {
    const name = uniqueName(names, s.name);
    names.push(name);
    return { ...s, name, patterns: [...s.patterns], id: newScopeId() };
  });
  return [...existing, ...added];
}
