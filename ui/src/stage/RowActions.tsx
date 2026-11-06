import { LoaderCircle, Minus, Plus, Trash2, type LucideIcon } from 'lucide-react';
import type { FileChange } from '../api/gen/FileChange';
import type { FileListPayload } from '../api/gen/FileListPayload';
import type { FileRow } from '../files/fileTree';
import { useRef } from 'react';
import { HoverTooltip } from '../ui/HoverTooltip';
import { useDisarmOnChange } from '../ui/arm/useDisarmOnChange';
import type { WriteCtx } from '../write/client';
import { pathsOf, stagePaths, unstageFiles } from './actions';
import { discardableOf, discardFiles } from './discardFiles';
import { COMMIT_QUEUED, DISCARDING, useCommitting, useDiscardingAny } from './store';
import './stage.css';

/** A row's files: the file itself, or every file under a tree-mode folder. */
export function filesUnder(list: FileListPayload, row: FileRow): FileChange[] {
  if (row.kind === 'file') return row.change ? [row.change] : [];
  const prefix = `${row.path}/`;
  return list.files.filter((f) => f.path.startsWith(prefix));
}

/** An icon button inside a row: a press never selects the row or moves the list's focus. */
export function ActionIcon({ label, tip, icon: Icon, disabled = false, pending = false, danger = false, positive = false, text, shortText, onClick }: { label: string; tip: string; icon: LucideIcon; disabled?: boolean; /** Its write is running: a spinner in the icon's place, and off. */ pending?: boolean; danger?: boolean; positive?: boolean; /** A text button; the icon alone shows when the panel is narrow. */ text?: string; /** A shorter label for a medium-width panel. */ shortText?: string; onClick: () => void }) {
  const off = disabled || pending;
  const Shown = pending ? LoaderCircle : Icon;
  return (
    <HoverTooltip content={tip}>
      <button
        type="button"
        className={`row-action${danger ? ' danger' : ''}${positive ? ' positive' : ''}${text ? ' has-text' : ''}`}
        aria-label={label}
        aria-disabled={off}
        aria-busy={pending || undefined}
        data-pending={pending ? '' : undefined}
        tabIndex={-1}
        onMouseDown={(e) => {
          e.preventDefault();
          e.stopPropagation();
        }}
        onClick={(e) => {
          e.stopPropagation();
          if (!off) onClick();
        }}
      >
        <Shown size={text ? 14 : 13} className={pending ? 'spin' : undefined} aria-hidden />
        {text && <span className="row-action-text">{text}</span>}
        {shortText && <span className="row-action-short">{shortText}</span>}
      </button>
    </HoverTooltip>
  );
}

/** A WIP row's hover actions (spec #2 §7.1): unstaged rows Discard and Stage,
 * staged rows Unstage. A folder row acts on every file under it. */
export function RowActions({ ctx, which, files, name }: { ctx: WriteCtx; which: 'unstaged' | 'staged'; files: FileChange[]; name: string }) {
  const committing = useCommitting(ctx.repoId, ctx.worktree);
  // A submodule's own changes are discarded inside it (T4): its row offers no Discard.
  const discardable = discardableOf(files);
  // Its files are being discarded: the Discard shows it runs, and neither button sends another write.
  const discarding = useDiscardingAny(ctx.repoId, ctx.worktree, pathsOf(discardable)) && which === 'unstaged';
  // An armed Discard names its files: another row in this place (a refresh) disarms it.
  const ref = useRef<HTMLSpanElement>(null);
  useDisarmOnChange(ref, `${ctx.worktree}\0${files.map((f) => f.path).join('\0')}`);
  if (files.length === 0) return null;
  // Discard arms in place, over the row's other actions (spec §ui confirms, board C).
  const discard = () => discardFiles(ctx, discardable, name);
  const tip = (t: string) => (committing ? COMMIT_QUEUED : discarding ? DISCARDING : t);
  return (
    <span ref={ref} className="row-actions" data-arm-cover="" data-arm-grow="left">
      {which === 'unstaged' && discardable.length > 0 && <ActionIcon label={`Discard ${name}`} tip={tip('Discard changes')} icon={Trash2} danger disabled={committing} pending={discarding} onClick={() => void discard()} />}
      {which === 'unstaged'
        ? <ActionIcon label={`Stage ${name}`} tip={tip('Stage')} icon={Plus} positive text="Stage File" disabled={committing || discarding} onClick={() => void stagePaths(ctx, pathsOf(files))} />
        : <ActionIcon label={`Unstage ${name}`} tip={committing ? COMMIT_QUEUED : 'Unstage'} icon={Minus} danger text="Unstage File" disabled={committing} onClick={() => void unstageFiles(ctx, files)} />}
    </span>
  );
}
