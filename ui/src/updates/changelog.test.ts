import { describe, expect, it } from 'vitest';
import bundled from 'virtual:gitvolt-changelog';
import { compareVersions, notesBetween, parseChangelog } from './changelog';

const FIXTURE = `# Changelog

Intro text.

## [Unreleased]

### Added

- **Not yet.** Unreleased work.

## [1.6.1] - 2026-11-12

### Fixed

- **Clone on Windows.** A drive path works (#26).

## [1.6.0] - 2026-11-06

### Added

- **Emoji completion.** In the commit summary.

## [1.6.0-rc.2] - 2026-11-04

- Second candidate.

## [1.6.0-rc.1] - 2026-11-03

- First candidate.

## Not a version

- Ignored.

## [1.5.1] - 2026-11-05

- A patch.

## [1.5.0] - 2026-11-01

- The base.

[1.5.0]: https://forge.example/compare/v1.4.0...v1.5.0
`;

describe('parseChangelog', () => {
  const entries = parseChangelog(FIXTURE);

  it('keeps released sections only, with their dates and trimmed bodies', () => {
    expect(entries.map((e) => e.version)).toEqual(['1.6.1', '1.6.0', '1.6.0-rc.2', '1.6.0-rc.1', '1.5.1', '1.5.0']);
    expect(entries[0]).toEqual({ version: '1.6.1', date: '2026-11-12', body: '### Fixed\n\n- **Clone on Windows.** A drive path works (#26).' });
    expect(entries.some((e) => e.body.includes('Not yet'))).toBe(false);
  });

  it('ends a section at any ## heading and drops trailing link references', () => {
    expect(entries.find((e) => e.version === '1.6.0-rc.1')?.body).toBe('- First candidate.');
    expect(entries.at(-1)?.body).toBe('- The base.');
  });

  it('copes with malformed headings and CRLF', () => {
    const text = '## [1.2]\n- bad version\n## [x.y.z] - 2026-01-01\n- bad\n## [1.0.0]\r\n- no date\r\n## [0.9.0] -\n- dash, no date\n';
    expect(parseChangelog(text)).toEqual([
      { version: '1.0.0', date: '', body: '- no date' },
      { version: '0.9.0', date: '', body: '- dash, no date' },
    ]);
    expect(parseChangelog('')).toEqual([]);
  });

  it("bundles the repository's own CHANGELOG.md: every released section, dated, with notes", () => {
    expect(bundled.length).toBeGreaterThan(10);
    expect(bundled.every((e) => /^\d+\.\d+\.\d+/.test(e.version) && /^\d{4}-\d{2}-\d{2}$/.test(e.date) && e.body.trim())).toBe(true);
    expect(bundled.map((e) => e.version)).not.toContain('Unreleased');
  });
});

describe('compareVersions', () => {
  it('orders as SemVer does, ignoring build metadata', () => {
    expect(compareVersions('1.6.1', '1.6.0')).toBeGreaterThan(0);
    expect(compareVersions('1.10.0', '1.9.9')).toBeGreaterThan(0);
    expect(compareVersions('1.6.0-rc.1', '1.6.0')).toBeLessThan(0);
    expect(compareVersions('1.6.0-rc.10', '1.6.0-rc.2')).toBeGreaterThan(0);
    expect(compareVersions('1.6.0-rc', '1.6.0-rc.1')).toBeLessThan(0);
    expect(compareVersions('1.6.0-1', '1.6.0-alpha')).toBeLessThan(0);
    expect(compareVersions('v1.6.1+202611121200.abc1234', '1.6.1')).toBe(0);
  });

  it("is null for what isn't a version", () => {
    expect(compareVersions('1.6', '1.6.0')).toBeNull();
    expect(compareVersions('1.06.0', '1.6.0')).toBeNull();
    expect(compareVersions('dev', '1.6.0')).toBeNull();
  });
});

describe('notesBetween', () => {
  const entries = parseChangelog(FIXTURE);
  const versions = (since: string | null, current: string) => notesBetween(entries, since, current).map((e) => e.version);

  it('every version after the last-run one up to the current one, newest first', () => {
    expect(versions('1.5.0', '1.6.1')).toEqual(['1.6.1', '1.6.0', '1.6.0-rc.2', '1.6.0-rc.1', '1.5.1']);
    expect(versions('1.6.0', '1.6.1')).toEqual(['1.6.1']);
  });

  it('from a pre-release: the later candidates and the release', () => {
    expect(versions('1.6.0-rc.1', '1.6.0')).toEqual(['1.6.0', '1.6.0-rc.2']);
  });

  it('leaves out versions newer than the running one, and sorts a file out of order', () => {
    expect(versions('1.5.0', '1.6.0')).toEqual(['1.6.0', '1.6.0-rc.2', '1.6.0-rc.1', '1.5.1']);
    const shuffled = [entries[4], entries[0], entries[2], entries[1]];
    expect(notesBetween(shuffled, '1.5.0', '1.6.1').map((e) => e.version)).toEqual(['1.6.1', '1.6.0', '1.6.0-rc.2', '1.5.1']);
  });

  it("no last-run version: the current version's notes alone; none when it has no section", () => {
    expect(versions(null, '1.6.1')).toEqual(['1.6.1']);
    expect(versions(null, '1.7.0')).toEqual([]);
    expect(versions('garbage', '1.6.1')).toEqual([]);
  });
});
