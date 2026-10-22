import { useTheme } from '../store';
import './editor.css';

/** A lane's line through a row, with its commit on it (and, on a merge, the merged lane coming in). */
function Lanes({ merge = false }: { merge?: boolean }) {
  return (
    <svg className="theme-preview-lanes" width="34" height="24" viewBox="0 0 34 24">
      <line x1="10" y1="0" x2="10" y2="24" strokeWidth="2" style={{ stroke: 'var(--graph-0)' }} />
      {merge && <path d="M10 12 C 10 20, 24 18, 24 24" strokeWidth="2" fill="none" style={{ stroke: 'var(--graph-1)' }} />}
      <circle cx="10" cy="12" r="4" strokeWidth="2" style={{ fill: 'var(--node-fill)', stroke: 'var(--graph-0)' }} />
    </svg>
  );
}

/**
 * A static slice of the app beside the colours: two graph rows (a ref chip, the selected row), a
 * changed file and three diff lines. It's drawn with the app's tokens, so it shows the applied
 * theme (the edited one), previews included, with nothing to update.
 */
export function LivePreview() {
  const chipText = useTheme((s) => s.colors.laneText[0]);
  return (
    <div className="theme-preview" aria-hidden>
      <div className="theme-preview-title">Live preview</div>
      <div className="theme-preview-graph">
        <div className="theme-preview-row">
          <span className="theme-preview-chip" style={{ background: 'var(--graph-0)', color: chipText }}>main</span>
          <Lanes />
          <span className="theme-preview-subject">Fuel gauge reads the tank</span>
        </div>
        <div className="theme-preview-row selected">
          <span />
          <Lanes merge />
          <span className="theme-preview-subject">Merge 'feature/trade-ui'</span>
        </div>
      </div>
      <div className="theme-preview-file">
        <b style={{ color: 'var(--status-modified)' }}>M</b>
        <span>economy.rs</span>
      </div>
      <div className="theme-preview-diff">
        <div className="theme-preview-line">{'  let base = cargo.value();'}</div>
        <div className="theme-preview-line" style={{ background: 'var(--diff-removed-line)' }}>{'- let fee = base * 0.9;'}</div>
        <div className="theme-preview-line" style={{ background: 'var(--diff-inserted-line)' }}>{'+ let fee = base * SELL_SHARE;'}</div>
      </div>
    </div>
  );
}
