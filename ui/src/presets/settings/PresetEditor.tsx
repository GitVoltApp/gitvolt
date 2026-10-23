import { useRef } from 'react';
import type { BranchPicks } from '../../api/gen/BranchPicks';
import type { BranchPreset } from '../../api/gen/BranchPreset';
import type { PresetPreview } from '../../api/gen/PresetPreview';
import type { PresetScope } from '../../api/gen/PresetScope';
import type { Rule } from '../../api/gen/Rule';
import { GROUP_COLORS } from '../../app/tabGroups';
import { FieldInput } from '../../fileColours/settings/FieldInput';
import { arm } from '../../ui/arm/store';
import { Switch } from '../../ui/Switch';
import { useSortable } from '../../ui/sortable/useSortable';
import type { PresetEntry } from '../model';
import { presetColour } from '../PresetButton';
import { OtherwiseRow, RuleRow } from './RuleRow';
import { useSortableKeys } from './sortableKeys';

export interface PresetEditorProps {
  entry: PresetEntry;
  /** The open repository's name; null without one (no picks, counts or "only here"). */
  repoName: string | null;
  picks: BranchPicks | null;
  /** The live counts for this preset as it is now; null while asked. */
  preview: PresetPreview | null;
  focusName: boolean;
  onEdit(fn: (p: BranchPreset) => BranchPreset): void;
  onScope(scope: PresetScope): void;
  onForget(name: string, which: keyof BranchPicks): void;
  onDuplicate(): void;
  onCopy(): void;
  onDelete(): void;
}

const move = <T,>(list: readonly T[], from: number, to: number): T[] => {
  const out = [...list];
  const [x] = out.splice(from, 1);
  out.splice(to, 0, x);
  return out;
};

/** The right column: one preset's name, where it's available, colour, rules, picks and options. */
export function PresetEditor({ entry, repoName, picks, preview, focusName, onEdit, onScope, onForget, onDuplicate, onCopy, onDelete }: PresetEditorProps) {
  const { preset: p, readOnly } = entry;
  const rules = useRef<HTMLDivElement>(null);
  const setRules = (next: Rule[]) => onEdit((x) => ({ ...x, rules: next }));
  const sortable = useSortable({ count: p.rules.length, axis: 'y', onMove: (from, to) => setRules(move(p.rules, from, to)), label: (i) => `Rule ${i + 1}` });
  useSortableKeys(rules, sortable);
  // Counts line up with the rules only when the answer is for this list of them.
  const counts = preview && preview.rules.length === p.rules.length + 1 ? preview.rules : null;

  const del = () => {
    void arm({ arm: `Delete ${p.name}`, tone: 'danger', title: `Delete ${p.name}?`, body: 'Repositories on it go back to All.', confirmLabel: `Delete ${p.name}` })
      .then((ok) => { if (ok) onDelete(); });
  };

  return (
    <div className="bp-editor" aria-label={`Preset ${p.name}`} role="group">
      <div className="bp-row bp-head">
        <label className="bp-field bp-name-field">
          <span className="bp-label">Name</span>
          {readOnly
            ? <input className="bp-input" value={p.name} readOnly aria-label="Name" />
            : <FieldInput key={p.id} className="bp-input" label="Name" value={p.name} autoFocus={focusName}
                onCommit={(v) => { const name = v.trim(); if (name && name !== p.name) onEdit((x) => ({ ...x, name })); }} />}
        </label>
        <div className="bp-field">
          <span className="bp-label" id="bp-scope-label">Available in</span>
          {readOnly ? <span className="bp-readonly">Built in, everywhere</span> : (
            <span className="bp-seg bp-scope" role="group" aria-labelledby="bp-scope-label">
              <button type="button" aria-pressed={p.scope === 'all'} onClick={() => p.scope !== 'all' && onScope('all')}>All repositories</button>
              {repoName !== null && <button type="button" aria-pressed={p.scope === 'repo'} onClick={() => p.scope !== 'repo' && onScope('repo')}>{repoName} only</button>}
            </span>
          )}
        </div>
      </div>

      <div className="bp-field">
        <span className="bp-label" id="bp-colour-label">Colour</span>
        <span className="bp-colours" role="group" aria-labelledby="bp-colour-label">
          {GROUP_COLORS.map((c) => (
            <button key={c} type="button" className="bp-colour" aria-label={c} aria-pressed={p.color === c} disabled={readOnly && p.color !== c}
              style={{ background: presetColour(c) }} onClick={() => { if (!readOnly && p.color !== c) onEdit((x) => ({ ...x, color: c })); }} />
          ))}
        </span>
      </div>

      <section className="bp-section" aria-labelledby="bp-rules-title">
        <h4 className="bp-h" id="bp-rules-title">Rules <span className="bp-hint">top to bottom; the first that matches a branch decides</span></h4>
        <div className="bp-rules">
          <div ref={rules} role="list" aria-label="Rules" {...sortable.containerProps}>
            {p.rules.map((r, i) => (
              <RuleRow key={i} index={i} rule={r} count={counts?.[i] ?? null} readOnly={readOnly}
                itemProps={readOnly ? undefined : sortable.itemProps(i)} handleProps={readOnly ? undefined : sortable.handleProps(i)}
                onChange={(next) => setRules(p.rules.map((x, j) => (j === i ? next : x)))}
                onRemove={() => setRules(p.rules.filter((_, j) => j !== i))} />
            ))}
          </div>
          <OtherwiseRow action={p.otherwise} count={counts?.[p.rules.length] ?? null} readOnly={readOnly} onChange={(otherwise) => onEdit((x) => ({ ...x, otherwise }))} />
        </div>
        <span className="bp-sr" aria-live="polite">{sortable.liveText}</span>
        {!readOnly && <button type="button" className="bp-link" onClick={() => setRules([...p.rules, { action: 'hide', matcher: { kind: 'nameMatches', patterns: '' } }])}>+ Add rule</button>}
      </section>

      {!readOnly && repoName !== null && (
        <section className="bp-section" aria-labelledby="bp-picks-title">
          <h4 className="bp-h" id="bp-picks-title">Picked by hand <span className="bp-hint">alt+click a branch chip; these come before the rules</span></h4>
          {picks && (picks.hidden.length || picks.shown.length) ? (
            <div className="bp-picks">
              {picks.hidden.length > 0 && <Chips label="Always hidden:" names={picks.hidden} onForget={(n) => onForget(n, 'hidden')} />}
              {picks.shown.length > 0 && <Chips label="Always shown:" names={picks.shown} onForget={(n) => onForget(n, 'shown')} />}
            </div>
          ) : <p className="bp-note">None in {repoName}.</p>}
        </section>
      )}

      <section className="bp-section" aria-labelledby="bp-options-title">
        <h4 className="bp-h" id="bp-options-title">Options</h4>
        <div className="bp-options">
          <Switch label="Fold branches merged into the trunk" description="into their merge commit; hover it to see them" checked={p.foldMerged} disabled={readOnly}
            onChange={(foldMerged) => onEdit((x) => ({ ...x, foldMerged }))} />
          <Switch label="Mark where hidden branches fork" description="not for branches picked by hand" checked={p.ghostLines} disabled={readOnly}
            onChange={(ghostLines) => onEdit((x) => ({ ...x, ghostLines }))} />
        </div>
      </section>

      {repoName !== null ? <Summary repoName={repoName} preview={preview} /> : <p className="bp-summary bp-note">Open a repository to see what this preset decides in it.</p>}

      <div className="bp-actions">
        <button type="button" className="bp-btn" onClick={onDuplicate}>Duplicate</button>
        <button type="button" className="bp-btn" onClick={onCopy}>Copy as JSON</button>
        <span className="bp-spacer" />
        {!readOnly && <button type="button" className="bp-btn bp-danger" onClick={del}>Delete preset</button>}
      </div>
    </div>
  );
}

function Chips({ label, names, onForget }: { label: string; names: string[]; onForget(name: string): void }) {
  return (
    <span className="bp-chips">
      <span className="bp-note">{label}</span>
      {names.map((n) => (
        <span key={n} className="bp-chip">{n}<button type="button" className="bp-chip-x" aria-label={`Forget ${n}`} onClick={() => onForget(n)}>✕</button></span>
      ))}
    </span>
  );
}

const num = (n: number | undefined) => (n === undefined ? '–' : String(n));

/** "In orbital now: 9 shown · 9 collapsed · 20 hidden · 14 merged branches folded". */
function Summary({ repoName, preview }: { repoName: string; preview: PresetPreview | null }) {
  const p = preview ?? undefined;
  return (
    <p className="bp-summary" aria-live="polite">
      In {repoName} now: <i className="bp-sum-dot" data-kind="shown" aria-hidden /><b>{num(p?.shown)}</b> shown · <i className="bp-sum-dot" data-kind="collapsed" aria-hidden /><b>{num(p?.collapsed)}</b> collapsed
      {' · '}<i className="bp-sum-dot" data-kind="hidden" aria-hidden /><b>{num(p?.hidden)}</b> hidden · <b>{num(p?.folded)}</b> merged branches folded
    </p>
  );
}
