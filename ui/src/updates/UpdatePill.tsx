import { ArrowDownToLine, LoaderCircle, RotateCw, TriangleAlert } from 'lucide-react';
import { useEffect } from 'react';
import { HoverTooltip } from '../ui/HoverTooltip';
import { percentOf, releaseOf, useUpdates } from './store';
import './updates.css';

/**
 * The status bar's update pill, left of the Keyboard Shortcuts button: shown only while there's
 * an update (so it takes no room otherwise), the same width in every state (no shift as it
 * moves on). Every state opens the update dialog: "Update to 0.3.0" (its notes, and Install and
 * restart); while that runs, a small progress bar (the percentage in its tooltip), then
 * "Installing 0.3.0". "Install 0.3.0" and "Restart to update" are left when it stopped short:
 * an install that couldn't run by itself, a restart that failed.
 */
export function UpdatePill() {
  const state = useUpdates((s) => s.state);
  const running = useUpdates((s) => s.running);
  useEffect(() => { useUpdates.getState().listen(); }, []);
  const release = releaseOf(state);
  if (!release) return null;
  const { openDialog } = useUpdates.getState();
  const v = release.version;
  switch (state.state) {
    case 'available':
      return (
        <HoverTooltip content={`GitVolt ${v} is available`}>
          <button type="button" className="sb-update" data-state="available" onClick={openDialog}>
            <ArrowDownToLine size={11} aria-hidden />Update to {v}
          </button>
        </HoverTooltip>
      );
    case 'downloading': {
      const pct = percentOf(state);
      return (
        <HoverTooltip content={`Downloading GitVolt ${v}: ${pct}%`}>
          <button type="button" className="sb-update" data-state="downloading" aria-label={`Downloading GitVolt ${v}, ${pct}%`} onClick={openDialog}>
            <span className="sb-update-bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct}><span style={{ width: `${pct}%` }} /></span>
          </button>
        </HoverTooltip>
      );
    }
    case 'ready':
      // Install and restart goes straight on to the install.
      if (!running) return <button type="button" className="sb-update" data-state="ready" onClick={openDialog}><ArrowDownToLine size={11} aria-hidden />Install {v}</button>;
    // falls through
    case 'installing':
      return <button type="button" className="sb-update" data-state="installing" onClick={openDialog}><LoaderCircle size={11} className="sb-spin" aria-hidden />Installing {v}</button>;
    case 'installed':
      return <button type="button" className="sb-update" data-state="installed" onClick={openDialog}><RotateCw size={11} aria-hidden />Restart to update</button>;
    case 'failed':
      return (
        <HoverTooltip content={state.message}>
          <button type="button" className="sb-update failed" data-state="failed" onClick={openDialog}><TriangleAlert size={11} aria-hidden />Update failed</button>
        </HoverTooltip>
      );
    default:
      return null;
  }
}
