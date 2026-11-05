import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { createPortal } from 'react-dom';
import { useRepoContext } from '../app/repoContext';
import { useAppState } from '../app/state';
import { typeInto, useCompletion, type Provider } from '../markdown/useCompletion';
import { flushDrafts, WIP_DRAFT_COUNTER_FROM, WIP_DRAFT_WARN_FROM, type WipDraft } from './draft';
import { expandGitmoji, findGitmojiTrigger, GITMOJI_MAX, loadGitmojis, rankGitmojis, type Gitmoji } from './gitmoji';
import './commit.css';
import { registerKeyHints } from '../shortcuts/hints';

const isSubmit = (e: KeyboardEvent) => e.key === 'Enter' && (e.ctrlKey || e.metaKey);

/** The gitmojis while the repository's setting is on (loaded once, as their own chunk), else
 * null; off outside a repository tab. */
function useGitmojis(): readonly Gitmoji[] | null {
  const { path } = useRepoContext();
  const on = useAppState((s) => !!path && !!s.profile.repos[path]?.gitmoji);
  const [list, setList] = useState<readonly Gitmoji[] | null>(null);
  useEffect(() => { if (on) void loadGitmojis().then(setList, () => undefined); }, [on]);
  return on ? list : null;
}

/** `:` in the summary or the description → the gitmojis; inserts the emoji and a space. */
function gitmojiProvider(list: readonly Gitmoji[]): Provider<Gitmoji> {
  return {
    label: 'Gitmoji',
    className: 'commit-gitmoji-pop',
    max: GITMOJI_MAX,
    find: findGitmojiTrigger,
    local: (q) => rankGitmojis(list, q),
    key: (g) => g.code,
    token: (g) => `${g.emoji} `,
    row: (g) => <><span className="md-emoji-glyph" aria-hidden>{g.emoji}</span><span className="commit-gitmoji-code">:{g.code}:</span><span className="commit-gitmoji-desc">{g.description}</span></>,
  };
}

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
 * With gitmoji on (Settings), a `:` in either field opens the gitmoji picker (↑/↓, Enter/Tab,
 * Esc closes it; `findGitmojiTrigger` says where), and a whole `:code:` typed becomes its emoji.
 */
export function CommitFields({ value, onChange, onSubmit, onEscape, disabled = false, autoFocus = false }: { value: WipDraft; onChange: (d: WipDraft) => void; onSubmit: () => void; onEscape?: () => void; disabled?: boolean; autoFocus?: boolean }) {
  const summary = useRef<HTMLInputElement>(null);
  const description = useRef<HTMLTextAreaElement>(null);
  const gitmojis = useGitmojis();
  const provider = useMemo(() => (gitmojis ? gitmojiProvider(gitmojis) : null), [gitmojis]);
  const setSummary = (text: string) => onChange({ ...value, summary: text });
  const setDescription = (text: string) => onChange({ ...value, description: text });
  const completion = useCompletion(summary, value.summary, setSummary, [provider]);
  const descCompletion = useCompletion(description, value.description, setDescription, [provider]);
  /** Where the caret goes once a `:code:` became its emoji (where the box can't type it). */
  const caretTo = useRef<{ box: HTMLInputElement | HTMLTextAreaElement; at: number; sync: () => void } | null>(null);
  /** A `:code:` just typed, to become its emoji once React has the text as typed. */
  const expandNext = useRef<{ box: HTMLInputElement | HTMLTextAreaElement; start: number; end: number; emoji: string } | null>(null);
  useLayoutEffect(() => {
    const c = caretTo.current;
    if (c !== null) {
      caretTo.current = null;
      c.box.setSelectionRange(c.at, c.at);
      c.sync();
    }
    const x = expandNext.current;
    if (x !== null) {
      expandNext.current = null;
      // Typed into the box (its input event comes back through `changed`), so Cmd+Z gives the
      // `:code:` back; after this commit, not inside it.
      queueMicrotask(() => { typeInto(x.box, x.start, x.end, x.emoji); });
    }
  });
  const changed = (box: HTMLInputElement | HTMLTextAreaElement, set: (text: string) => void, sync: () => void) => {
    const x = gitmojis && expandGitmoji(box.value, box.selectionStart ?? box.value.length, gitmojis);
    const typed = x !== null && typeof document.execCommand === 'function';
    if (x && typed) expandNext.current = { box, start: x.start, end: x.end, emoji: x.emoji };
    else if (x) caretTo.current = { box, at: x.caret, sync };
    set(x && !typed ? x.text : box.value);
    sync();
  };
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
          onChange={(e) => changed(e.target, setSummary, completion.sync)}
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
        onChange={(e) => changed(e.target, setDescription, descCompletion.sync)}
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
  { id: 'key.gitmoji', section: 'Commit message', label: 'Pick a gitmoji', keys: [':'], context: "(in the message, with gitmoji on in the repository's settings)", source: 'commit/CommitFields.tsx' },
  { id: 'key.msgBlur', section: 'Commit message', label: 'Leave the message input', keys: ['Esc'], context: '(when in message input)', source: 'commit/CommitFields.tsx' },
]);
