import { lazy, Suspense, useEffect } from 'react';
import { useModalKeys } from '../app/modalKeys';
import { checkLastRun, useWhatsNew } from './whatsNew';
import './whatsNew.css';

// The changelog and the Markdown renderer load with the panel, never at startup.
const WhatsNew = lazy(() => import('./WhatsNew'));

const close = () => useWhatsNew.getState().close();

/**
 * What's new (the bell's "Updated to GitVolt …", Help → What's new): the bundled changelog's notes
 * from the version that last ran up to this one, newest first. Mounted with the app, it also
 * notices an update at startup (`checkLastRun`).
 */
export function WhatsNewDialog() {
  const range = useWhatsNew((s) => s.open);
  const ref = useModalKeys<HTMLDivElement>(range !== null, close);
  useEffect(() => { void checkLastRun().catch((e: unknown) => console.warn('[gitvolt] last run version', e)); }, []);
  if (!range) return null;
  return (
    <div className="modal-backdrop" onPointerDown={close}>
      <div ref={ref} className="modal whats-new-dialog" role="dialog" aria-modal="true" aria-label="What's new" onPointerDown={(e) => e.stopPropagation()}>
        <h2>What&apos;s new in GitVolt {range.version}</h2>
        {range.since && <p className="whats-new-sub">Every change since {range.since}, the version you used before.</p>}
        <div className="whats-new-notes" tabIndex={0} aria-label="Release notes">
          <Suspense fallback={<p aria-busy="true">Loading…</p>}><WhatsNew range={range} /></Suspense>
        </div>
        <div className="modal-actions">
          <button type="button" autoFocus onClick={close}>Close</button>
        </div>
      </div>
    </div>
  );
}
