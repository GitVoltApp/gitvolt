import { describe, expect, it } from 'vitest';
import { exportFileName, MAX_SHARE_BYTES, parseShareJson, toShareJson, uniqueName } from './share';

describe('share envelope', () => {
  it('round-trips a body', () => {
    const text = toShareJson('theme', 1, { name: 'Nord', gitvolt: 'x', version: 9 });
    expect(parseShareJson(text, 'theme', 1)).toEqual({ ok: true, version: 1, body: { name: 'Nord' } });
  });

  it('refuses what is not this kind of envelope', () => {
    const bad = (t: string, kind: 'theme' | 'branch-preset' = 'theme', max = 1) => parseShareJson(t, kind, max).ok;
    expect(bad('x'.repeat(MAX_SHARE_BYTES + 1))).toBe(false);
    expect(bad('{nope')).toBe(false);
    expect(bad('[1]')).toBe(false);
    expect(bad('null')).toBe(false);
    expect(bad('"theme"')).toBe(false);
    expect(bad(toShareJson('branch-preset', 1, {}))).toBe(false);
    expect(bad('{"gitvolt":"theme"}')).toBe(false);
    expect(bad('{"gitvolt":"theme","version":1.5}')).toBe(false);
    expect(bad('{"gitvolt":"theme","version":"1"}')).toBe(false);
    expect(bad(toShareJson('theme', 2, {}))).toBe(false);
  });

  it('tolerates whitespace and a json fence', () => {
    const text = toShareJson('theme', 1, { name: 'A' });
    expect(parseShareJson(`  \n${text}\n `, 'theme', 1).ok).toBe(true);
    expect(parseShareJson('```json\n' + text + '\n```', 'theme', 1).ok).toBe(true);
    expect(parseShareJson('```\n' + text + '\n```\n', 'theme', 1).ok).toBe(true);
  });
});

describe('uniqueName', () => {
  it('suffixes a clash, without case', () => {
    expect(uniqueName('Nord', [])).toBe('Nord');
    expect(uniqueName('Nord (Custom)', ['nord (custom)'])).toBe('Nord (Custom) (2)');
    expect(uniqueName('Nord', ['Nord', 'Nord (2)'])).toBe('Nord (3)');
  });
  it('bumps a name already ending in (n) from its base', () => {
    expect(uniqueName('Nord (2)', ['Nord', 'Nord (2)'])).toBe('Nord (3)');
  });
});

describe('exportFileName', () => {
  it('slugs the name', () => {
    expect(exportFileName('Nord (Custom)', 'theme')).toBe('nord-custom.gitvolt-theme.json');
    expect(exportFileName('!!!', 'branch-preset')).toBe('branch-preset.gitvolt-branch-preset.json');
  });
});
