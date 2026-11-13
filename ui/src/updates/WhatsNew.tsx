import { useMemo } from 'react';
import changelog from 'virtual:gitvolt-changelog';
import { Markdown } from '../markdown/Markdown';
import { notesBetween, type ChangelogEntry } from './changelog';
import type { WhatsNewRange } from './whatsNew';

/** No tab's forge behind it: links open in the browser, references stay text. */
const CONTEXT = { kind: 'forge', tabId: 'gitvolt-whats-new' } as const;

/** `Nov 12, 2026` for a section's `2026-11-12`; empty when it has none. */
function day(date: string): string {
  const d = /^\d{4}-\d{2}-\d{2}$/.test(date) ? new Date(`${date}T00:00:00Z`) : null;
  return d && !Number.isNaN(d.getTime()) ? d.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' }) : '';
}

/** The versions' notes, each under its version heading (the sections' own `###` one level down). */
export function WhatsNewNotes({ entries }: { entries: ChangelogEntry[] }) {
  if (!entries.length) return <p className="whats-new-empty">No notes for this version.</p>;
  return (
    <>
      {entries.map((e) => (
        <section key={e.version} className="whats-new-version" aria-label={`GitVolt ${e.version}`}>
          <h3>{e.version}{e.date && <span className="whats-new-date"> · {day(e.date)}</span>}</h3>
          <Markdown text={e.body.replace(/^###(?= )/gm, '####')} flavor="github" context={CONTEXT} />
        </section>
      ))}
    </>
  );
}

/** What's new's body: the notes for `range` from the CHANGELOG.md bundled at build time, loaded with the panel. */
export default function WhatsNew({ range }: { range: WhatsNewRange }) {
  const entries = useMemo(() => notesBetween(changelog, range.since, range.version), [range.since, range.version]);
  return <WhatsNewNotes entries={entries} />;
}
