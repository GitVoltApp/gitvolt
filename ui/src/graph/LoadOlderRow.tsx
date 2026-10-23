import { forwardRef, type KeyboardEvent } from 'react';

interface Props { start: number; rowH: number; loaded: number; batch: number; loading: boolean; onLoad: () => void; onKeyDown: (e: KeyboardEvent<HTMLButtonElement>) => void }

/** The row after the last one of a truncated graph: loads the next commit-limit step. */
export const LoadOlderRow = forwardRef<HTMLButtonElement, Props>(function LoadOlderRow({ start, rowH, loaded, batch, loading, onLoad, onKeyDown }, ref) {
  return (
    <div className="load-older-row" style={{ top: start, height: rowH }}>
      <button ref={ref} type="button" className="load-older-btn" disabled={loading} onClick={onLoad} onKeyDown={onKeyDown}>
        Load older commits
        <span className="load-older-hint">{loading ? 'Loading…' : `${batch.toLocaleString()} more`}</span>
      </button>
      <span className="load-older-count">{loaded.toLocaleString()} commits loaded</span>
    </div>
  );
});
