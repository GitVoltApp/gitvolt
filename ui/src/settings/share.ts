/* The JSON envelope settings are shared in (themes, branch presets, file colours): pasted text is
   untrusted, so this only checks the envelope; each kind validates its own body. */

export type ShareKind = 'theme' | 'branch-preset' | 'file-colours';

/** Pasted texts over this are not settings. */
export const MAX_SHARE_BYTES = 65_536;

export function toShareJson(kind: ShareKind, version: number, body: Record<string, unknown>): string {
  const { gitvolt: _k, version: _v, ...rest } = body;
  return JSON.stringify({ gitvolt: kind, version, ...rest }, null, 2);
}

/** The envelope's body (everything but `gitvolt` and `version`), or `ok: false` for anything that isn't one. */
export function parseShareJson(
  text: string,
  kind: ShareKind,
  maxVersion: number,
): { ok: true; body: Record<string, unknown>; version: number } | { ok: false } {
  if (new TextEncoder().encode(text).length > MAX_SHARE_BYTES) return { ok: false };
  // A ```json fence around it (pasted from chat) is tolerated.
  const raw = text.trim().replace(/^```(?:json)?[ \t]*\r?\n?/i, '').replace(/\r?\n?```$/, '').trim();
  let parsed: unknown;
  try { parsed = JSON.parse(raw); } catch { return { ok: false }; }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return { ok: false };
  const { gitvolt, version, ...body } = parsed as Record<string, unknown>;
  if (gitvolt !== kind) return { ok: false };
  if (typeof version !== 'number' || !Number.isInteger(version) || version < 1 || version > maxVersion) return { ok: false };
  return { ok: true, body, version };
}

/** `name`, else `"name (2)"`, `"name (3)"`…; compared without case. A name already ending in `(n)` is bumped from its base. */
export function uniqueName(name: string, taken: Iterable<string>): string {
  const used = new Set([...taken].map((t) => t.toLowerCase()));
  if (!used.has(name.toLowerCase())) return name;
  const base = name.replace(/ \(\d+\)$/, '');
  for (let n = 2; ; n++) {
    const candidate = `${base} (${n})`;
    if (!used.has(candidate.toLowerCase())) return candidate;
  }
}

/** `"<slug>.gitvolt-<kind>.json"`. */
export function exportFileName(name: string, kind: ShareKind): string {
  const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '');
  return `${slug || kind}.gitvolt-${kind}.json`;
}
