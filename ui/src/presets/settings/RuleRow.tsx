import { GripVertical } from 'lucide-react';
import type { HTMLAttributes } from 'react';
import type { Matcher } from '../../api/gen/Matcher';
import type { PresetAction } from '../../api/gen/PresetAction';
import type { Rule } from '../../api/gen/Rule';
import { FieldInput } from '../../fileColours/settings/FieldInput';
import { Select } from '../../ui/Select';

type Kind = Matcher['kind'];
export const MATCHERS: ReadonlyArray<readonly [Kind, string]> = [
  ['trunk', 'is the trunk'], ['checkedOut', 'is checked out'], ['commitsBy', 'has commits by'], ['nameMatches', 'name matches'],
  ['remote', 'is remote'], ['local', 'is local'], ['olderThan', 'last commit older than'], ['mergedIntoTrunk', 'is merged into the trunk'],
];
const ACTIONS: ReadonlyArray<readonly [PresetAction, string]> = [['show', 'Show'], ['hide', 'Hide'], ['collapse', 'Collapse']];

/** A matcher of `kind`, with a starting value for its field. */
export function matcherOf(kind: Kind, old: Matcher): Matcher {
  if (kind === old.kind) return old;
  switch (kind) {
    case 'commitsBy': return { kind, author: { kind: 'me' } };
    case 'nameMatches': return { kind, patterns: '' };
    case 'olderThan': return { kind, days: 90 };
    default: return { kind };
  }
}

export const countText = (n: number | null | undefined) => (n == null ? '' : `${n} branch${n === 1 ? '' : 'es'}`);

/** Show / Hide / Collapse. */
export function ActionChoice({ value, label, readOnly, onChange }: { value: PresetAction; label: string; readOnly: boolean; onChange(a: PresetAction): void }) {
  return (
    <span className="bp-seg" role="group" aria-label={label}>
      {ACTIONS.map(([a, text]) => (
        <button key={a} type="button" data-action={a} aria-pressed={value === a} disabled={readOnly && value !== a} onClick={() => { if (!readOnly) onChange(a); }}>{text}</button>
      ))}
    </span>
  );
}

const Note = ({ children }: { children: string }) => <span className="bp-note">{children}</span>;

/** The matcher's own field: patterns, days, an author. */
function MatcherField({ m, n, readOnly, onChange }: { m: Matcher; n: number; readOnly: boolean; onChange(m: Matcher): void }) {
  switch (m.kind) {
    case 'checkedOut': return <Note>in any worktree; always shown, even when a rule hides it</Note>;
    case 'nameMatches':
      return readOnly ? <code className="bp-code">{m.patterns}</code> : (
        <FieldInput className="bp-input bp-code" label={`Rule ${n} patterns`} placeholder="dependabot/*, renovate/*" value={m.patterns}
          onCommit={(v) => { if (v.trim() !== m.patterns) onChange({ ...m, patterns: v.trim() }); }} />
      );
    case 'olderThan':
      return (
        <>
          {readOnly ? <span>{m.days}</span> : (
            <FieldInput className="bp-input bp-days" label={`Rule ${n} days`} value={String(m.days)}
              onCommit={(v) => { const d = Math.round(Number(v)); if (v.trim() !== '' && Number.isFinite(d) && d >= 0 && d !== m.days) onChange({ ...m, days: d }); }} />
          )}
          <span>days</span><Note>in the loaded commits</Note>
        </>
      );
    case 'commitsBy': {
      const who = m.author.kind === 'person' ? m.author.who : '';
      return (
        <>
          {readOnly ? <span>{m.author.kind === 'me' ? 'me' : who}</span> : (
            <>
              <Select<'me' | 'person'> aria-label={`Rule ${n} author`} value={m.author.kind} options={[['me', 'me'], ['person', 'someone']]}
                onChange={(k) => onChange({ ...m, author: k === 'me' ? { kind: 'me' } : { kind: 'person', who } })} />
              {m.author.kind === 'person' && (
                <FieldInput className="bp-input" label={`Rule ${n} author's name or email`} placeholder="name or email" value={who}
                  onCommit={(v) => { if (v.trim() !== who) onChange({ ...m, author: { kind: 'person', who: v.trim() } }); }} />
              )}
            </>
          )}
          <Note>in the loaded commits</Note>
        </>
      );
    }
    default: return null;
  }
}

export interface RuleRowProps {
  index: number;
  rule: Rule;
  count: number | null;
  readOnly: boolean;
  itemProps?: HTMLAttributes<HTMLElement>;
  handleProps?: HTMLAttributes<HTMLElement>;
  onChange(rule: Rule): void;
  onRemove(): void;
}

/** One rule: drag grip, its number, the action, what it matches, how many branches it decides, ✕. */
export function RuleRow({ index, rule, count, readOnly, itemProps, handleProps, onChange, onRemove }: RuleRowProps) {
  const n = index + 1;
  const label = MATCHERS.find(([k]) => k === rule.matcher.kind)?.[1] ?? rule.matcher.kind;
  return (
    <div role="listitem" className="bp-rule" aria-label={`Rule ${n}`} {...itemProps}>
      {readOnly ? <span className="bp-grip" aria-hidden /> : <button type="button" className="bp-grip" aria-label={`Reorder rule ${n}`} {...handleProps}><GripVertical size={14} aria-hidden /></button>}
      <span className="bp-rule-n">{n}</span>
      <ActionChoice value={rule.action} label={`Rule ${n} action`} readOnly={readOnly} onChange={(action) => onChange({ ...rule, action })} />
      <span className="bp-rule-what">
        {readOnly ? <span className="bp-matcher">{label}</span> : (
          <span className="bp-matcher">
            <Select<Kind> aria-label={`Rule ${n} matches`} value={rule.matcher.kind} options={MATCHERS} onChange={(k) => onChange({ ...rule, matcher: matcherOf(k, rule.matcher) })} />
          </span>
        )}
        <MatcherField m={rule.matcher} n={n} readOnly={readOnly} onChange={(matcher) => onChange({ ...rule, matcher })} />
      </span>
      <span className="bp-count">{countText(count)}</span>
      {readOnly ? <span className="bp-x" aria-hidden /> : <button type="button" className="bp-x" aria-label={`Remove rule ${n}`} onClick={onRemove}>✕</button>}
    </div>
  );
}

/** The last word: what a branch no rule matches gets. */
export function OtherwiseRow({ action, count, readOnly, onChange }: { action: PresetAction; count: number | null; readOnly: boolean; onChange(a: PresetAction): void }) {
  return (
    <div className="bp-rule bp-otherwise">
      <span className="bp-grip" aria-hidden />
      <span className="bp-rule-n" />
      <ActionChoice value={action} label="Everything else action" readOnly={readOnly} onChange={onChange} />
      <span className="bp-rule-what">Everything else</span>
      <span className="bp-count">{countText(count)}</span>
      <span className="bp-x" aria-hidden />
    </div>
  );
}
