// Reached only through the center view's React.lazy (history/feature.ts): it pulls in File View,
// the Monaco loader and Shiki's language registry, which stay out of the startup chunk.
import { X } from 'lucide-react';
import { defaultRangeExtractor, useVirtualizer, type Range } from '@tanstack/react-virtual';
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type MouseEvent, type ReactNode } from 'react';
import { useStore, type StoreApi } from 'zustand';
import { api } from '../api/client';
import { copyText } from '../api/transport';
import type { FileHistoryRow } from '../api/gen/FileHistoryRow';
import type { DiffTarget } from '../repo/store';
import { selectCommit } from '../app/graphNav';
import { tabStore } from '../app/tabStores';
import { Avatar } from '../avatars/Avatar';
import { editorOwnsEscape, ESCAPE_OWNER_AREAS, useContents } from '../diff/DiffPanel';
import { useDiffPrefs, type HistoryView } from '../diff/diffPrefs';
import { DiffToolbar } from '../diff/DiffToolbar';
import { FileView } from '../diff/FileView';
import { fileSideOf, HexBody, HexView } from '../diff/hex';
import { hexOf } from '../diff/hexContents';
import { LfsPlaceholder, lfsMissing } from '../lfs/LfsPlaceholder';
import { highlightLanguage } from '../diff/language';
import { isPdf } from '../pdf/kind';
import { PdfBody } from '../pdf/PdfBody';
import { relativeTime } from '../format/relative';
import { shortSha } from '../format/sha';
import { useCenterViewEditorFile, type CenterViewProps } from '../repo/centerView';
import { PanelResizer } from '../repo/PanelResizer';
import { fileViewTarget, useRepoView } from '../repo/store';
import { HoverTooltip } from '../ui/HoverTooltip';
import { registerKeys } from '../ui/keyRouter';
import { isEditableTarget } from '../ui/keys';
import { useToast } from '../ui/toastStore';
import { BlameLayer } from './BlameGutter';
import { ChangesAtCommit } from './HistoryChanges';
import { NO_BINARY_BLAME, NO_LFS_BLAME } from './HistoryButtons';
import { historyEnd, selectedRow, type FileHistoryArgs } from './model';
import { clampListW, LIST_W, loadListW, saveListW } from './listWidth';
import { createHistoryStore, HISTORY_PAGE, type HistoryStore } from './store';
import './history.css';

/** The Blame toggle's tooltip in Changes, where it's off: blame is the file's, not the diff's. */
export const NO_CHANGES_BLAME = 'Blame shows on File: pick File to see who last changed each line';

/** File History's `File | Changes` switch, in the toolbar's centre where Diff View has File View /
 * Diff View. The pick is a diff pref, so it holds from row to row and from one history to the next. */
function HistoryViewToggle({ view }: { view: HistoryView }) {
  const set = useDiffPrefs((s) => s.set);
  return (
    <div className="segmented" role="group" aria-label="History view">
      <button type="button" aria-pressed={view === 'file'} onClick={() => set({ historyView: 'file' })}>File</button>
      <button type="button" aria-pressed={view === 'changes'} onClick={() => set({ historyView: 'changes' })}>Changes</button>
    </div>
  );
}

/**
 * File History, in the graph's place: the file's commits on the left (newest
 * first, paged), the file at the selected commit on the right, read-only, or the changes that
 * commit made to it (`File | Changes`, a remembered pick). × or Esc (from
 * anywhere in the tab, unless a text box or one of Monaco's own overlays has it) closes it.
 */
export function FileHistory({ tabId, props, close }: CenterViewProps<FileHistoryArgs>) {
  const [store] = useState(() => createHistoryStore(props, (skip) => api.fileHistory(props.repoId, props.worktree, props.path, props.rev, skip, HISTORY_PAGE)));
  const row = useStore(store, selectedRow);
  const ref = useRef<HTMLElement>(null);
  const blame = useStore(store, (s) => s.blame);
  // The selected file at its commit is a binary: shown as hex, with no lines to blame (no toggle).
  const [binary, setBinary] = useState<false | 'binary' | 'lfs'>(false);
  const onBinary = useCallback((b: boolean, lfs?: boolean) => setBinary(lfs ? 'lfs' : b ? 'binary' : false), []);
  const view = useDiffPrefs((s) => s.prefs.historyView);
  const changes = view === 'changes';
  // Blame is the file's: off (in place, saying why) for a binary and in Changes.
  const noBlame = binary === 'lfs' ? NO_LFS_BLAME : binary ? NO_BINARY_BLAME : changes ? NO_CHANGES_BLAME : null;
  const [listW, setListW] = useState(loadListW);
  const changeListW = useCallback((w: number) => { setListW(w); saveListW(w); }, []);
  // The drag writes the columns straight to the section (rAF-coalesced by the resizer); React state
  // and storage see the width once, at the end of the gesture.
  const liveColumns = useCallback((w: number) => ref.current?.style.setProperty('--fh-list', `${w}px`), []);
  // A right-click in the file: its menu (Copy path, the forge permalink, Open in…) is the selected
  // row's file at its commit, not the open file this view may hide.
  const shown = row && row.status !== 'D' ? row : null;
  const editorFile = useMemo(() => (shown ? { target: rowTarget(shown.path, shown.sha), root: props.worktree } : null), [shown, props.worktree]);
  const fileTarget = useMemo(() => (row ? rowTarget(row.path, row.sha) : null), [row]);
  const views = <HistoryViewToggle view={view} />;
  useCenterViewEditorFile(tabId, editorFile);
  // A blame group's commit opens in this list (pages load until it shows); Alt+click
  // selects it in the graph instead, which the view closes to show.
  const onPick = useCallback((sha: string, inGraph: boolean) => {
    if (inGraph) {
      close();
      if (!selectCommit(tabId, sha, { focus: true })) useToast.getState().show('Not in the loaded history');
      return;
    }
    void store.getState().seek(sha).then((found) => { if (!found) useToast.getState().show(`${shortSha(sha)} isn't in this file's history`); });
  }, [close, store, tabId]);
  useEffect(() => { void store.getState().loadMore(); }, [store]);
  // The core reads the pages from one `git log` walk: closed (or hidden), the view lets it go. A
  // page asked for later walks again.
  useEffect(() => () => { api.fileHistoryClose(props.repoId, props.worktree, props.path, props.rev).catch(() => {}); }, [props.repoId, props.worktree, props.path, props.rev]);
  // The tab's sticky mode follows the Blame toggle: the next file opens as this was left.
  useEffect(() => store.subscribe((s, prev) => {
    const tab = tabStore(tabId)?.getState();
    if (s.blame !== prev.blame && tab?.stickyHistory) tab.setStickyHistory({ blame: s.blame });
  }), [store, tabId]);
  useEffect(() => registerKeys('app', (e) => {
    if (e.key !== 'Escape' || e.ctrlKey || e.altKey || e.metaKey || e.shiftKey || e.isComposing || e.defaultPrevented) return;
    if (ref.current?.checkVisibility?.() === false) return;
    const t = e.target instanceof Element ? e.target : null;
    if (isEditableTarget(t) && !t?.closest('.monaco-editor, .monaco-host')) return;
    if (e.composedPath().some((n) => n instanceof Element && n.matches(ESCAPE_OWNER_AREAS)) && editorOwnsEscape()) return;
    e.preventDefault();
    close();
    return 'handled';
  }), [close]);
  return (
    <section ref={ref} className="file-history" role="region" aria-label="File history" style={{ '--fh-list': `${clampListW(listW)}px` } as CSSProperties}>
      {/* The bar has the body's columns: the title over the list, Blame over the editor. */}
      <header className="file-history-header">
        <h2 className="file-history-title">File History: <span className="file-history-path">{props.path}</span></h2>
        <div className="file-history-tools">
          {/* A binary or Changes keeps the toggle, disabled, so the bar (and Close) never moves between files. */}
          <HoverTooltip content={noBlame ?? (blame ? 'Hide who last changed each line' : 'Show who last changed each line')}>
            <button type="button" className="blame-toggle" aria-pressed={!noBlame && blame} aria-disabled={!!noBlame || undefined} onClick={() => { if (!noBlame) store.getState().setBlame(!blame); }}>Blame</button>
          </HoverTooltip>
          <HoverTooltip content="Close (Esc)">
            <button type="button" className="icon-button" aria-label="Close file history" onClick={close}><X size={14} /></button>
          </HoverTooltip>
        </div>
      </header>
      <div className="file-history-body">
        <HistoryList store={store} path={props.path} follow={props.follow === true} />
        <PanelResizer className="fh-resizer" label="Resize commit list" grows="right" width={listW} defaultWidth={LIST_W.default} min={LIST_W.min} max={LIST_W.max} onChange={changeListW} onLive={liveColumns} />
        {/* Both views have Diff View's toolbar, the switch at its centre, so switching moves nothing. */}
        <div className="file-history-file">
          {row && fileTarget && (changes
            ? <ChangesAtCommit row={row} views={views} />
            : (
              <>
                <DiffToolbar target={fileTarget} canDiff canStep={false} binary={binary === 'binary'} views={views} />
                <div className="diff-body">
                  <FileAtCommit row={row} onBinary={onBinary}>{blame && <BlameLayer repoId={props.repoId} worktree={props.worktree} row={row} onPick={onPick} />}</FileAtCommit>
                </div>
              </>
            ))}
        </div>
      </div>
    </section>
  );
}

/** Copies the full hash without selecting the row (the click stops here). */
function copySha(e: MouseEvent, sha: string) {
  e.stopPropagation();
  const toast = useToast.getState().show;
  copyText(sha).then(() => toast('Copied'), () => toast('Copy failed'));
}

/** A commit row's height: two lines (the summary, then the date and author) and the padding. */
export const FH_ROW_H = 42;

/** One commit of the list: memoized, so a step re-renders the two rows whose selection changed. */
const HistoryRow = memo(function HistoryRow({ row, selected, top, onSelect }: { row: FileHistoryRow; selected: boolean; top: number; onSelect: (sha: string) => void }) {
  return (
    <li id={`fh-${row.sha}`} data-sha={row.sha} role="option" aria-selected={selected} className="file-history-row" style={{ transform: `translateY(${top}px)` }} onClick={() => onSelect(row.sha)}>
      <Avatar name={row.author} email={row.email} size={20} />
      <span className="fh-summary">{row.summary}</span>
      <HoverTooltip content={`Copy ${row.sha}`}>
        <button type="button" tabIndex={-1} className="fh-sha" aria-label={`Copy ${row.sha}`} onMouseDown={(e) => e.preventDefault()} onClick={(e) => copySha(e, row.sha)}>{shortSha(row.sha)}</button>
      </HoverTooltip>
      <span className="fh-meta">{relativeTime(row.time)} · {row.author}</span>
    </li>
  );
});

/** The commits, virtualized: only the rows on screen (and a few around them) are in the DOM, so
 * a long history's steps and pages cost what a short one's do. */
function HistoryList({ store, path, follow }: { store: StoreApi<HistoryStore>; path: string; follow: boolean }) {
  const rows = useStore(store, (s) => s.rows);
  const selected = useStore(store, (s) => s.selected);
  const more = useStore(store, (s) => s.more);
  const loading = useStore(store, (s) => s.loading);
  const error = useStore(store, (s) => s.error);
  const end = historyEnd({ rows, more, loading, error });
  const scrollRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const selectedIndex = useMemo(() => (selected === null ? -1 : rows.findIndex((r) => r.sha === selected)), [rows, selected]);
  // The selected row is always rendered, scrolled off or not: it's the listbox's active descendant.
  const rangeExtractor = useCallback((range: Range) => {
    const out = defaultRangeExtractor(range);
    return selectedIndex < 0 || out.includes(selectedIndex) ? out : [...out, selectedIndex].sort((a, b) => a - b);
  }, [selectedIndex]);
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => scrollRef.current, estimateSize: () => FH_ROW_H, overscan: 10, rangeExtractor, initialRect: { width: 360, height: 600 } });
  const onSelect = useCallback((sha: string) => store.getState().select(sha), [store]);
  // The keyboard moves in with the view: from the diff toolbar's History (whose mousedown keeps
  // focus in the file list, where ↓ would open the next file over the view), a menu or the
  // palette. Effects re-run when the view shows again (`<Activity>`), so a file peeked over it
  // closing hands the keyboard back here, not to the hidden graph. Except a file picked while
  // sticky (`follow`): the keyboard stays in the file list it was picked in, so ↑/↓ go on
  // stepping files.
  useEffect(() => {
    if (follow && document.activeElement?.closest('.file-list')) return;
    listRef.current?.focus({ preventScroll: true });
  }, [follow]);
  // The selection in view, from ↑/↓ or a blame click that loaded pages until it showed.
  useLayoutEffect(() => {
    if (selectedIndex >= 0) v.scrollToIndex(selectedIndex, { align: 'auto' });
  }, [selectedIndex, v]);
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    e.preventDefault();
    store.getState().step(e.key === 'ArrowDown' ? 1 : -1);
  };
  return (
    <div ref={scrollRef} className="file-history-list">
      <ul ref={listRef} role="listbox" aria-label="Commits" tabIndex={0} onKeyDown={onKeyDown} aria-activedescendant={selected ? `fh-${selected}` : undefined} style={{ height: v.getTotalSize(), '--fh-row': `${FH_ROW_H}px` } as CSSProperties}>
        {v.getVirtualItems().map((item) => {
          const r = rows[item.index];
          return <HistoryRow key={r.sha} row={r} selected={r.sha === selected} top={item.start} onSelect={onSelect} />;
        })}
      </ul>
      {error && (
        <div role="alert" className="file-history-note">
          Couldn't load the history: {error} <button type="button" className="inline-retry" onClick={() => void store.getState().loadMore()}>Retry</button>
        </div>
      )}
      {more && !error && (
        <button type="button" className="file-history-more" disabled={loading} onClick={() => void store.getState().loadMore()}>{loading ? 'Loading…' : 'Load more'}</button>
      )}
      {end && (
        <div className="file-history-end">
          {rows.length === 0 && <div>No commit changed {path}</div>}
          {end.addedIn && (
            <div>
              Added in{' '}
              <HoverTooltip content={`Copy ${end.addedIn}`}>
                <button type="button" tabIndex={-1} className="fh-sha" aria-label={`Copy ${end.addedIn}`} onMouseDown={(e) => e.preventDefault()} onClick={(e) => copySha(e, end.addedIn!)}>{shortSha(end.addedIn)}</button>
              </HoverTooltip>
            </div>
          )}
          <div>End of history</div>
        </div>
      )}
    </div>
  );
}

/** The file `path` at commit `sha`, as File View would open it. */
const rowTarget = (path: string, sha: string): DiffTarget => fileViewTarget(path, sha, { kind: 'commit', id: sha, parent: 0 });

/** The file at `row`'s commit, at that commit's path (a rename's old name below it). `children`
 * is laid over a shown text file (the blame gutter). A binary shows File View's hex panes
 * instead, and a PDF its pages, with no `children`: blame has no lines there. `onBinary`: whether the shown
 * file is one, once its contents are in (false again when none is shown). */
export function FileAtCommit({ row, children, onBinary }: { row: FileHistoryRow; children?: ReactNode; onBinary?: (binary: boolean, lfs?: boolean) => void }) {
  if (row.status === 'D') return <div className="diff-message"><p>{row.path} was deleted in this commit</p></div>;
  return <FileText row={row} onBinary={onBinary}>{children}</FileText>;
}

function FileText({ row, children, onBinary }: { row: FileHistoryRow; children?: ReactNode; onBinary?: (binary: boolean, lfs?: boolean) => void }) {
  const services = useRepoView((s) => s.services);
  const target = useMemo(() => rowTarget(row.path, row.sha), [row.path, row.sha]);
  const [forced, setForced] = useState<string | null>(null);
  const contents = useContents(services, target, forced === target.key);
  const text = contents.status === 'ready' ? contents.data.new?.text ?? null : null;
  const language = useMemo(() => (text === null ? 'plaintext' : highlightLanguage(row.path, text)), [row.path, text]);
  // Reported once the contents are in, so stepping between two binaries keeps the toggle away. A
  // PDF is one, even written in plain text: it shows as pages.
  const binary = contents.status === 'ready' && !contents.data.tooLarge ? text === null || isPdf(contents.data) : null;
  // An LFS file's lines are its pointer's to git: no blame either.
  const lfs = contents.status === 'ready' && !!contents.data.lfs;
  const report = useRef(onBinary);
  report.current = onBinary;
  useEffect(() => { if (binary !== null) report.current?.(binary, lfs); }, [binary, lfs]);
  useEffect(() => () => report.current?.(false), []);
  if (contents.status === 'error') return <div className="diff-message"><div role="alert">Couldn't load {row.path}: {contents.message}</div></div>;
  if (contents.status !== 'ready') return <div className="diff-message" aria-busy="true" />;
  if (lfsMissing(contents.data).length) return <LfsPlaceholder target={target} contents={contents.data} sides={{ old: null, new: row.sha }} />;
  if (contents.data.tooLarge) {
    return (
      <div className="diff-message">
        <p>{row.path} is too large to show at once.</p>
        <button type="button" className="text-button" onClick={() => setForced(target.key)}>Load anyway</button>
      </div>
    );
  }
  if (isPdf(contents.data)) return <PdfBody target={target} contents={contents.data} />;
  if (text === null) {
    // A binary (an image too): its hex at this commit. A non-image's dumps come with its contents
    // (hexContents.ts); an image's load here.
    const c = contents.data;
    const hex = hexOf(c);
    return hex ? <HexView path={row.path} hex={hex} file side={fileSideOf(c, hex)} /> : <HexBody target={target} contents={c} />;
  }
  return (
    <>
      <FileView identity={`history|${target.key}`} path={row.path} text={text} language={language} />
      {children}
    </>
  );
}
