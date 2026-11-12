import type { VersionNotes } from '../api/gen/VersionNotes';
import { Markdown } from '../markdown/Markdown';
import './updates.css';

/** No tab's forge behind it: links open in the browser, references stay text. */
const CONTEXT = { kind: 'forge', tabId: 'gitvolt-update' } as const;

/** `Oct 19, 2026` for a release's publish time; empty when it has none. UTC, as the release's day. */
function releaseDay(iso: string | null): string {
  const d = iso ? new Date(iso) : null;
  return d && !Number.isNaN(d.getTime()) ? d.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' }) : '';
}

/**
 * Release notes (GitHub-flavoured Markdown) of one or several versions, as given (the core sends
 * them newest first). One version has no divider; several each get a muted label (not a heading:
 * the bodies have their own). Loaded with whatever shows it, never at startup.
 */
export default function ReleaseNotes({ notes }: { notes: readonly VersionNotes[] }) {
  if (!notes.some((n) => n.notes.trim())) return <p className="update-notes-empty">No release notes.</p>;
  const many = notes.length > 1;
  return (
    <>
      {notes.map((n) => (
        <section key={n.version} className="update-version">
          {many && <div className="update-version-label">{[n.version, releaseDay(n.publishedAt)].filter(Boolean).join(' · ')}</div>}
          {n.notes.trim() ? <Markdown text={n.notes} flavor="github" context={CONTEXT} /> : <p className="update-notes-empty">No release notes.</p>}
        </section>
      ))}
    </>
  );
}
