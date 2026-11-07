import { useMemo, useRef, type KeyboardEvent } from 'react';
import { createPortal } from 'react-dom';
import { emojiProvider, useEmojiLists } from '../markdown/providers';
import { useCompletion } from '../markdown/useCompletion';
import { flushDrafts, WIP_DRAFT_COUNTER_FROM, WIP_DRAFT_WARN_FROM, type WipDraft } from './draft';
import './commit.css';
import { registerKeyHints } from '../shortcuts/hints';

const isSubmit = (e: KeyboardEvent) => e.key === 'Enter' && (e.ctrlKey || e.metaKey);

/**
 * The summary line and the description below it (spec #2 §8.1), the WIP draft's two fields.
 * - Enter or ↓ in the summary moves to the description (caret at its start).
 * - ↑ with the caret on the description's first line moves back (caret at the summary's end).
 * - Ctrl+Enter submits from either; Esc blurs, keeping the text (or `onEscape`, e.g. Cancel).
 * One text box (ux round 1): one border around both, the focus ring on it. The description
 * shows 4 lines, grows to 8, then scrolls (CSS `field-sizing`).
 * Keys typed here never reach the page's own handlers (the same rule as the WIP row's box); the
 * app's Ctrl chords still come first (key router), and the app-wide Commit (Ctrl+Enter,
 * `commit/keyActions.ts`) yields to these fields.
 * A `:` in either field opens the emoji completion, the Markdown fields' own (`emojiProvider`:
 * the gitmojis first, then the other emojis); a pick inserts the emoji itself. A `:code:` typed in
 * full stays as typed: GitHub and GitLab render it, and some teams write their gitmojis that way.
 */
export function CommitFields({ value, onChange, onSubmit, onEscape, disabled = false, autoFocus = false }: { value: WipDraft; onChange: (d: WipDraft) => void; onSubmit: () => void; onEscape?: () => void; disabled?: boolean; autoFocus?: boolean }) {
  const summary = useRef<HTMLInputElement>(null);
  const description = useRef<HTMLTextAreaElement>(null);
  const lists = useEmojiLists();
  const provider = useMemo(() => emojiProvider(lists, 'emoji'), [lists]);
  const setSummary = (text: string) => onChange({ ...value, summary: text });
  const setDescription = (text: string) => onChange({ ...value, description: text });
  const completion = useCompletion(summary, value.summary, setSummary, [provider]);
  const descCompletion = useCompletion(description, value.description, setDescription, [provider]);
  const escape = (e: KeyboardEvent<HTMLElement>) => {
    e.preventDefault();
    if (onEscape) onEscape();
    else {
      e.currentTarget.blur();
      flushDrafts();
    }
  };
  const onSummaryKey = (e: KeyboardEvent<HTMLInputElement>) => {
    e.stopPropagation();
    if (completion.keyDown(e)) return;
    if (e.nativeEvent.isComposing || e.keyCode === 229) return; // the input method's Enter or arrow
    if (isSubmit(e)) {
      e.preventDefault();
      onSubmit();
    } else if (e.key === 'Enter' || e.key === 'ArrowDown') {
      e.preventDefault();
      description.current?.focus();
      description.current?.setSelectionRange(0, 0);
    } else if (e.key === 'Escape') escape(e);
  };
  const onDescriptionKey = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    e.stopPropagation();
    if (descCompletion.keyDown(e)) return;
    if (e.nativeEvent.isComposing || e.keyCode === 229) return; // the input method's Enter or arrow
    const t = e.currentTarget;
    if (isSubmit(e)) {
      e.preventDefault();
      onSubmit();
    } else if (e.key === 'ArrowUp' && t.selectionStart === t.selectionEnd && !t.value.slice(0, t.selectionStart).includes('\n')) {
      e.preventDefault();
      const s = summary.current;
      s?.focus();
      s?.setSelectionRange(s.value.length, s.value.length);
    } else if (e.key === 'Escape') escape(e);
  };
  const n = value.summary.length;
  return (
    <div className="commit-fields">
      <div className="commit-summary-row">
        <input
          ref={summary}
          className="commit-summary"
          type="text"
          aria-label="Commit summary"
          placeholder="Summary"
          spellCheck
          autoComplete="off"
          autoFocus={autoFocus}
          disabled={disabled}
          value={value.summary}
          {...completion.aria}
          onChange={(e) => { setSummary(e.target.value); completion.sync(); }}
          onSelect={completion.sync}
          onKeyDown={onSummaryKey}
          onBlur={flushDrafts}
        />
        {/* On the body: the details panel is a size container, which fixed positioning goes by. */}
        {completion.popup && createPortal(completion.popup, document.body)}
        {n > WIP_DRAFT_COUNTER_FROM && <span className={`commit-counter${n > WIP_DRAFT_WARN_FROM ? ' warn' : ''}`} data-testid="commit-counter">{n}</span>}
      </div>
      <textarea
        ref={description}
        className="commit-description"
        aria-label="Commit description"
        placeholder="Description"
        spellCheck
        disabled={disabled}
        value={value.description}
        {...descCompletion.aria}
        onChange={(e) => { setDescription(e.target.value); descCompletion.sync(); }}
        onSelect={descCompletion.sync}
        onKeyDown={onDescriptionKey}
        onBlur={flushDrafts}
      />
      {descCompletion.popup && createPortal(descCompletion.popup, document.body)}
    </div>
  );
}

// Shown in the Keyboard Shortcuts panel (Ctrl+/); metadata only.
registerKeyHints([
  { id: 'key.commit', section: 'Commit message', label: 'Commit', keys: ['Mod+Enter'], context: '(when in message input)', source: 'commit/CommitFields.tsx' },
  { id: 'key.msgNext', section: 'Commit message', label: 'Summary to description', keys: ['Enter'], context: '(when in the summary)', source: 'commit/CommitFields.tsx' },
  { id: 'key.gitmoji', section: 'Commit message', label: 'Pick an emoji (gitmojis first)', keys: [':'], context: '(in the message)', source: 'commit/CommitFields.tsx' },
  { id: 'key.msgBlur', section: 'Commit message', label: 'Leave the message input', keys: ['Esc'], context: '(when in message input)', source: 'commit/CommitFields.tsx' },
]);
