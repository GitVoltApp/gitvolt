// git stash as git 2.53 does it (observed from the binary, never its source): the W/I/U commits
// written by hand so their ids are git's, the list as refs/stash's reflog, and apply's order of
// steps (index part, untracked files, then the tree merge), side effects of a refusal included.
// Not as git does: paths are literal (git's are pathspecs, globs included); the abbreviated id in
// the messages is always 7 digits (git lengthens it in a large repository); the index part's hunks
// come from the read engine's Myers diff, so among repeated lines one may sit where git's wouldn't.
import type { GbError } from '../../../api/gen/GbError';
import { gbError, gitFailure } from '../errors';
import { parentOf } from '../memfs';
import { readCommitHeaders } from '../objects';
import { compareBytewise, isBinaryHead, latin1 } from '../read/bytes';
import { AttrReader, markerSizeOf } from '../read/attributes';
import { gitSubject, readCommit } from '../read/commitText';
import type { IndexEntry, IndexStat } from '../read/indexFile';
import { diffLines, splitLines, unifiedHunks, type DiffHunk } from '../read/lineDiff';
import { fail } from '../read/seam';
import { diffTrees, listFiles, MODE_GITLINK, MODE_LINK, readBlobBytes, readRaw, type FileEntry, type TreeChange } from '../read/trees';
import { checkoutEntry, fileAt, kind, readFileState, removeWorktreePath, repoWithin, throughLink } from './files';
import { keepStat, loadIndex, saveIndex, stage0Of, unmergedOf } from './index';
import { conflictCount, mergeLines } from './mergeFile';
import { applyMerge, mergeTrees } from './mergeTrees';
import { worktreeProblems } from './mergeRun';
import { hardReset, mixedReset, refusal } from './move';
import { writeBlobBytes, writeCommitObject, writeIndexTree, writeTreeOf } from './objects';
import { latin1Bytes } from './patchApply';
import { casRefs, readHead, readRef, ZERO_OID } from './refs';
import type { WriteWt } from './seam';
import { othersUnder } from './snapshot';
import { statusEntries } from './status';

export const STASH_REF = 'refs/stash';
const LOG = (wt: WriteWt) => `${wt.demo.gitdir}/logs/${STASH_REF}`;

export function userMessage(subject: string): string {
  for (const prefix of ['On ', 'WIP on ']) {
    if (!subject.startsWith(prefix)) continue;
    const at = subject.indexOf(': ', prefix.length);
    if (at >= 0) return subject.slice(at + 2);
  }
  return subject;
}

/** git's whitespace here is space, tab, LF and CR (a vertical tab or form feed stays). */
export const reflogMessage = (text: string): string => text.replace(/[ \t\n\r]+/g, ' ').replace(/^ | $/g, '');

export interface StashEntry {
  oid: string;
  subject: string;
}

export function stashList(wt: WriteWt): StashEntry[] {
  return wt.demo.stashes().map((s) => ({ oid: s.id, subject: s.message }));
}

export function subjectOf(wt: WriteWt, oid: string): string {
  return stashList(wt).find((s) => s.oid === oid)?.subject ?? fail('NotFound', 'That stash is gone');
}

/** The reflog's lines, oldest first, as text. */
function logLines(wt: WriteWt): string[] {
  try {
    return (wt.fs.readFileSync(LOG(wt), 'utf8') as string).split('\n').filter((l) => l !== '');
  } catch {
    return [];
  }
}

function writeList(wt: WriteWt, lines: string[]): void {
  if (lines.length === 0) {
    if (wt.fs.existsSync(LOG(wt), false)) wt.fs.unlinkSync(LOG(wt));
    const now = readRef(wt.demo, STASH_REF);
    if (now !== null) casRefs(wt.demo, [{ name: STASH_REF, old: now, new: null }]);
    return;
  }
  wt.fs.mkdirpSync(parentOf(LOG(wt)));
  wt.fs.writeFileSync(LOG(wt), `${lines.join('\n')}\n`);
  const top = lines[lines.length - 1].split(' ')[1];
  wt.fs.mkdirpSync(`${wt.demo.gitdir}/refs`);
  wt.fs.writeFileSync(`${wt.demo.gitdir}/${STASH_REF}`, `${top}\n`);
}

export function storeStash(wt: WriteWt, ident: string, oid: string, message: string): void {
  const old = readRef(wt.demo, STASH_REF);
  // git logs nothing for a store of the stash already on top.
  if (old === oid) return;
  writeList(wt, [...logLines(wt), `${old ?? ZERO_OID} ${oid} ${ident}\t${reflogMessage(message)}`]);
}

export function dropStash(wt: WriteWt, oid: string): void {
  const lines = logLines(wt);
  // stash@{n} is the n-th line from the end; the newest one naming `oid`.
  const at = lines.map((l) => l.split(' ')[1]).lastIndexOf(oid);
  if (at < 0) fail('NotFound', 'That stash is gone');
  const [dropped] = lines.splice(at, 1);
  if (at < lines.length) {
    const next = lines[at].split(' ');
    next[0] = dropped.split(' ')[0];
    lines[at] = next.join(' ');
  }
  writeList(wt, lines);
}

export type Untracked = 'none' | 'untracked' | 'all';

const inScope = (paths: string[] | null) => (p: string) => paths === null || paths.some((q) => p === q || p.startsWith(`${q}/`));

/** Every file or link the index doesn't have, ignored ones too (`ls-files --others`). */
function others(wt: WriteWt, indexed: Set<string>): string[] {
  const out: string[] = [];
  for (const name of wt.fs.readdirSync(wt.root) as string[]) {
    if (name === '.git' || indexed.has(name)) continue;
    if (kind(wt, `${wt.root}/${name}`) === 'dir') out.push(...othersUnder(wt, name, indexed));
    else out.push(name);
  }
  return out.sort(compareBytewise);
}

/** `git clean -d`'s folders: each one holding no file at any depth goes (a submodule's stays). */
function removeEmptyFolders(wt: WriteWt, indexed: Set<string>): void {
  const walk = (rel: string): boolean => {
    let empty = true;
    for (const name of wt.fs.readdirSync(rel === '' ? wt.root : `${wt.root}/${rel}`) as string[]) {
      const p = rel === '' ? name : `${rel}/${name}`;
      if (p === '.git' || kind(wt, `${wt.root}/${p}`) !== 'dir' || indexed.has(p) || !walk(p)) empty = false;
    }
    if (empty && rel !== '') wt.fs.rmdirSync(`${wt.root}/${rel}`);
    return empty;
  };
  walk('');
}

const NO_INITIAL = 'You do not have the initial commit yet';

/** `narrow`: a WIP file's or folder's Stash (the core's `narrow`): I and W hold the changes in
 *  `paths` only, not the whole index git records; an autostash's keeps git's. `keepStaged`: an
 *  Unstaged part's (the core's `keep_staged`): the scope's staged changes go back after the reset,
 *  staged and in the files. */
export async function pushStash(wt: WriteWt, ident: string, message: string, o: { untracked: Untracked; paths: string[] | null; narrow?: boolean; keepStaged?: boolean }): Promise<StashEntry | null> {
  const head = readHead(wt);
  if (head.oid === null) throw gitFailure(NO_INITIAL, `${NO_INITIAL}\n`);
  const entries = await loadIndex(wt);
  // git's own words; it prints the `<path>: needs merge` lines on stdout.
  if (unmergedOf(entries).size > 0) throw gitFailure('could not write index', 'error: could not write index\n');
  const scope = inScope(o.paths);
  const status = await statusEntries(wt.repo, wt.root);
  const tracked = status.filter((s) => s.kind !== 'untracked' && (scope(s.path) || (s.origPath !== null && scope(s.origPath))));
  const indexed = new Set(entries.map((e) => e.path));
  const untracked =
    o.untracked === 'none' ? [] : o.untracked === 'all' ? others(wt, indexed).filter(scope) : status.filter((s) => s.kind === 'untracked' && scope(s.path)).map((s) => s.path);
  // Nothing to stash: no change in scope (an intent-to-add entry whose file is gone is one).
  if (tracked.length === 0 && untracked.length === 0) return null;
  // An intent-to-add entry whose file is there stops git before it writes anything (in scope or not).
  const blocked = entries.find((e) => e.intentToAdd && fileAt(wt, e.path) !== null);
  if (blocked) {
    const m = `Entry '${blocked.path}' not uptodate. Cannot merge.`;
    throw gitFailure(m, `error: ${m}\nCannot save the current worktree state\n`);
  }
  const index = stage0Of(entries.filter((e) => !e.intentToAdd));
  const headFiles = await listFiles(wt.repo, head.oid);
  // W: the index, with the paths in scope that HEAD or the index has taken from the worktree; an
  // addition whose file is gone keeps its staged blob, a staged deletion whose file is there comes
  // back, as git's do. A submodule keeps its entry.
  const changed = new Set(status.filter((s) => s.kind !== 'untracked' && s.worktree !== '.').map((s) => s.path));
  const w = new Map([...index].map(([p, e]) => [p, { path: p, mode: e.mode, oid: e.oid }]));
  for (const p of new Set([...headFiles.keys(), ...index.keys()])) {
    const e = index.get(p);
    if (!scope(p) || e?.mode === MODE_GITLINK || (e && !changed.has(p))) continue;
    const f = fileAt(wt, p);
    if (f !== null) w.set(p, { path: p, mode: f.mode, oid: await writeBlobBytes(wt.repo, f.bytes) });
    else if (headFiles.has(p)) w.delete(p);
  }
  const c = await readCommit(wt.repo, head.oid);
  const branch = head.branch ?? '(no branch)';
  const on = `${branch}: ${head.oid.slice(0, 7)} ${gitSubject(c.rawMessage)}`;
  // Narrowed: HEAD's files with the scope's entries of the index (I) or of W.
  const narrowed = async (from: Map<string, FileEntry>) => {
    const files = new Map(headFiles);
    for (const p of new Set([...headFiles.keys(), ...from.keys()])) {
      if (!scope(p)) continue;
      const e = from.get(p);
      if (e) files.set(p, e);
      else files.delete(p);
    }
    return writeTreeOf(wt.repo, [...files].map(([path, e]) => ({ path, mode: e.mode, oid: e.oid })));
  };
  const iTree = o.narrow ? await narrowed(index) : await writeIndexTree(wt.repo, entries);
  const i = await writeCommitObject(wt.repo, { tree: iTree, parents: [head.oid], author: ident, committer: ident, message: `index on ${on}\n` });
  const parents = [head.oid, i];
  // git 2.53 writes the untracked files' commit whenever -u or -a is given, with none in it too.
  if (o.untracked !== 'none') {
    const files = [];
    for (const p of untracked) {
      const f = readFileState(wt, p)!;
      files.push({ path: p, mode: f.mode, oid: await writeBlobBytes(wt.repo, f.bytes) });
    }
    parents.push(await writeCommitObject(wt.repo, { tree: await writeTreeOf(wt.repo, files), parents: [], author: ident, committer: ident, message: `untracked files on ${on}\n` }));
  }
  const full = message === '' ? `WIP on ${on}` : `On ${branch}: ${message}`;
  const oid = await writeCommitObject(wt.repo, { tree: o.narrow ? await narrowed(w) : await writeTreeOf(wt.repo, w.values()), parents, author: ident, committer: ident, message: full });
  // A multi-line message lists by its first line (the core's relist: a drop and a store again).
  const first = message.split('\n')[0].replace(/\r$/, '').trim();
  const subject = reflogMessage(message.trimEnd().includes('\n') ? `On ${branch}: ${first}` : full);
  storeStash(wt, ident, oid, subject);
  // The untracked files go first: one HEAD has (its entry removed from the index) comes back.
  // Their folders go when that empties them; without paths, every empty folder (git's `clean -d`).
  for (const p of untracked) removeWorktreePath(wt, p);
  if (o.paths === null) {
    if (o.untracked !== 'none') removeEmptyFolders(wt, indexed);
    await hardReset(wt, head.oid);
  } else {
    // Only the paths in scope go back to HEAD, in the index and the worktree.
    await mixedReset(wt, head.oid, o.paths);
    const stats = new Map<string, IndexStat>();
    for (const p of new Set(tracked.flatMap((s) => [s.path, ...(s.origPath === null ? [] : [s.origPath])]))) {
      if (!scope(p)) continue;
      const e = headFiles.get(p);
      if (!e) removeWorktreePath(wt, p);
      else {
        const st = await checkoutEntry(wt, p, e);
        if (st) stats.set(p, st);
      }
    }
    await saveIndex(wt, (await loadIndex(wt)).map((e) => (stats.has(e.path) && e.stage === 0 ? { ...e, stat: stats.get(e.path)! } : e)));
    if (o.keepStaged) await checkoutStaged(wt, [...new Set([...headFiles.keys(), ...index.keys()])].filter((p) => scope(p) && !sameEntry(headFiles.get(p) ?? null, index.get(p) ?? null)), index);
  }
  return { oid, subject };
}

const sameEntry = (a: FileEntry | null, b: FileEntry | null) => (a === null ? b === null : b !== null && a.oid === b.oid && a.mode === b.mode);

/** `git checkout --no-overlay <index part> -- <paths>`: each path's entry of `from`, in the index
 *  and the file (none: both go). */
async function checkoutStaged(wt: WriteWt, paths: string[], from: Map<string, FileEntry>): Promise<void> {
  if (paths.length === 0) return;
  const current = stage0Of(await loadIndex(wt));
  const next = (await loadIndex(wt)).filter((e) => !paths.includes(e.path));
  for (const p of paths) {
    const e = from.get(p);
    if (!e) {
      removeWorktreePath(wt, p);
      continue;
    }
    const st = await checkoutEntry(wt, p, e);
    const entry = keepStat(current, { path: p, mode: e.mode, oid: e.oid, stage: 0 });
    next.push(st ? { ...entry, stat: st } : entry);
  }
  await saveIndex(wt, next);
}

/** Why one file's part can't be taken out of it, or put into it, alone (the core's `Clash`). */
type Clash = 'lines' | 'binary' | 'link' | 'kind' | 'submodule' | 'whole';

function pick<T>(eq: (a: T, b: T) => boolean, ours: T, base: T, theirs: T): { v: T } | null {
  if (eq(ours, base)) return { v: theirs };
  if (eq(theirs, base) || eq(ours, theirs)) return { v: ours };
  return null;
}

/** The core's `merged`: the 3-way merge of one file's entries, `ours` with what changed from
 *  `base` to `theirs` (the exec bit too); contents by a line merge, never a relocated hunk. */
async function mergeEntry(wt: WriteWt, ours: FileEntry | null, base: FileEntry | null, theirs: FileEntry | null): Promise<{ entry: FileEntry | null } | { clash: Clash }> {
  const whole = pick(sameEntry, ours, base, theirs);
  if (whole) return { entry: whole.v };
  if (!ours || !base || !theirs) return { clash: 'whole' };
  const kind = (m: number) => (m === MODE_GITLINK ? 2 : m === MODE_LINK ? 1 : 0);
  if ([ours, base, theirs].some((e) => kind(e.mode) === 2)) return { clash: 'submodule' };
  if (kind(ours.mode) !== kind(base.mode) || kind(theirs.mode) !== kind(base.mode)) return { clash: 'kind' };
  const mode = pick((a, b) => a === b, ours.mode, base.mode, theirs.mode)!.v;
  const oid = pick((a, b) => a === b, ours.oid, base.oid, theirs.oid);
  if (oid) return { entry: { mode, oid: oid.v } };
  if (kind(base.mode) === 1) return { clash: 'link' };
  const [o, b, t] = [await readBlobBytes(wt.repo, ours.oid), await readBlobBytes(wt.repo, base.oid), await readBlobBytes(wt.repo, theirs.oid)];
  if ([o, b, t].some(isBinaryHead)) return { clash: 'binary' };
  const chunks = mergeLines(splitLines(latin1(b)), splitLines(latin1(o)), splitLines(latin1(t)), 'merge');
  if (conflictCount(chunks) > 0) return { clash: 'lines' };
  const text = chunks.flatMap((c) => (c.kind === 'common' ? c.lines : [])).join('');
  return { entry: { mode, oid: await writeBlobBytes(wt.repo, latin1Bytes(text)) } };
}

/** The core's `refusal`: why a Staged part's file refuses. */
async function partRefusal(wt: WriteWt, path: string, staged: boolean, inHead: boolean, clash: Clash): Promise<GbError> {
  if (clash === 'submodule') return gbError('Conflict', `Can't stash the staged changes alone: ${path} is a submodule. Nothing was stashed`);
  let why: string;
  if (clash === 'lines') why = `${path} has unstaged edits on or next to the same lines`;
  else if (clash === 'binary') why = `${path} is a binary file with unstaged edits`;
  else if (clash === 'link') why = `${path} is a symbolic link with an unstaged change`;
  else if (clash === 'kind') why = `${path} has an unstaged change between a file and a symbolic link`;
  else if (!staged) why = `${path} is staged as deleted, but there's a file there again`;
  else if (!inHead) {
    const renamed = (await statusEntries(wt.repo, wt.root)).some((e) => e.path === path && e.origPath !== null);
    why = `${path} is ${renamed ? 'renamed' : 'a new file'} with unstaged edits, which would have no file to stay in`;
  } else why = `${path} is deleted, unstaged`;
  return gbError('Conflict', `Can't stash the staged changes alone: ${why}. Stash its unstaged changes first, or stage them. Nothing was stashed`);
}

/** A file as an entry (its blob written), or null when there's none. */
async function fileEntry(wt: WriteWt, p: string): Promise<FileEntry | null> {
  const f = fileAt(wt, p);
  return f === null ? null : { mode: f.mode, oid: await writeBlobBytes(wt.repo, f.bytes) };
}

/** A Staged part (the core's `push_staged`): the scope's staged changes as the stash's index and
 *  worktree parts, on HEAD; each file is merged from its staged version to HEAD's, keeping its
 *  unstaged edits, and unstaged. Every file is merged first: a clash refuses, nothing stashed. */
export async function pushStaged(wt: WriteWt, ident: string, message: string, paths: string[]): Promise<StashEntry | null> {
  const head = readHead(wt);
  if (head.oid === null) throw gbError('InvalidInput', NO_INITIAL);
  const entries = await loadIndex(wt);
  if (unmergedOf(entries).size > 0) throw gbError('InvalidInput', 'Resolve the conflicted files first');
  const scope = inScope(paths);
  const index = stage0Of(entries.filter((e) => !e.intentToAdd));
  const headFiles = await listFiles(wt.repo, head.oid);
  const staged = [...new Set([...headFiles.keys(), ...index.keys()])].filter((p) => scope(p) && !sameEntry(headFiles.get(p) ?? null, index.get(p) ?? null)).sort(compareBytewise);
  if (staged.length === 0) return null;
  // Each file without its staged part: merged from the staged version to HEAD's.
  const left = new Map<string, FileEntry | null>();
  for (const p of staged) {
    const [h, s, u] = [headFiles.get(p) ?? null, index.get(p) ?? null, await fileEntry(wt, p)];
    const out = await mergeEntry(wt, u, s, h);
    if ('clash' in out) throw await partRefusal(wt, p, s !== null, h !== null, out.clash);
    left.set(p, out.entry);
  }
  const files = new Map(headFiles);
  for (const p of staged) {
    const e = index.get(p);
    if (e) files.set(p, { mode: e.mode, oid: e.oid });
    else files.delete(p);
  }
  const tree = await writeTreeOf(wt.repo, [...files].map(([path, e]) => ({ path, mode: e.mode, oid: e.oid })));
  const branch = head.branch ?? '(no branch)';
  const on = `${branch}: ${head.oid.slice(0, 7)} ${gitSubject((await readCommit(wt.repo, head.oid)).rawMessage)}`;
  const i = await writeCommitObject(wt.repo, { tree, parents: [head.oid], author: ident, committer: ident, message: `index on ${on}\n` });
  const oid = await writeCommitObject(wt.repo, { tree, parents: [head.oid, i], author: ident, committer: ident, message: `On ${branch}: ${message}` });
  const subject = reflogMessage(`On ${branch}: ${message.split('\n')[0].replace(/\r$/, '').trim()}`);
  storeStash(wt, ident, oid, subject);
  for (const [p, e] of left) {
    if (e === null) removeWorktreePath(wt, p);
    else await checkoutEntry(wt, p, e);
  }
  await mixedReset(wt, head.oid, staged);
  return { oid, subject };
}

/** One text patch onto `target` as `git apply` places it, no fuzz: each hunk's old lines found at
 *  their place or the nearest offset (forward first), in any order; a hunk from the first line
 *  only at the start, one with no context after it only at the end, none over lines an earlier hunk
 *  wrote. Null when a hunk doesn't apply. */
function applyHunks(target: string[], hunks: DiffHunk[]): string[] | null {
  const out = [...target];
  // The lines an earlier hunk wrote (its context too): a later one may not match over them.
  const patched = target.map(() => false);
  let shift = 0;
  for (const h of hunks) {
    const pre = h.lines.filter((l) => l.kind !== 'add').map((l) => l.text);
    const post = h.lines.filter((l) => l.kind !== 'del').map((l) => l.text);
    const atStart = h.oldStart <= 1;
    const atEnd = h.lines[h.lines.length - 1]?.kind !== 'context';
    const want = Math.max(0, h.oldStart - 1 + shift);
    const fits = (at: number) =>
      at >= 0 && at + pre.length <= out.length && (!atStart || at === 0) && (!atEnd || at + pre.length === out.length) && pre.every((l, k) => out[at + k] === l && !patched[at + k]);
    let at = -1;
    for (let d = 0; at < 0 && (want - d >= 0 || want + d <= out.length); d++) {
      if (fits(want + d)) at = want + d;
      else if (fits(want - d)) at = want - d;
    }
    if (at < 0) return null;
    out.splice(at, pre.length, ...post);
    patched.splice(at, pre.length, ...post.map(() => true));
    shift += post.length - pre.length + (at - want);
  }
  return out;
}

/** One path's change `from` → `to` (git's HEAD→index patch: an addition, a deletion, or a text
 *  patch with 3 lines of context) applied onto `onto` as `git apply --cached` would; null when it
 *  doesn't apply. The autostash's prediction (Task 20) asks the same of the target's entry. */
export async function patchEntry(wt: WriteWt, from: FileEntry | null, to: FileEntry | null, onto: FileEntry | null): Promise<{ entry: FileEntry | null } | null> {
  const same = (a: FileEntry | null, b: FileEntry | null) => (a === null ? b === null : b !== null && a.oid === b.oid && a.mode === b.mode);
  if (from === null) return onto === null ? { entry: to } : null;
  if (to === null) return same(from, onto) ? { entry: null } : null;
  if (same(from, onto)) return { entry: to };
  if (onto === null || from.mode === MODE_GITLINK) return null;
  const [ob, nb, tb] = [await readBlobBytes(wt.repo, from.oid), await readBlobBytes(wt.repo, to.oid), await readBlobBytes(wt.repo, onto.oid)];
  if (isBinaryHead(ob) || isBinaryHead(nb) || isBinaryHead(tb)) return null;
  const [ol, nl] = [splitLines(latin1(ob)), splitLines(latin1(nb))];
  const applied = applyHunks(splitLines(latin1(tb)), unifiedHunks(ol, nl, diffLines(ol, nl)));
  if (applied === null) return null;
  return { entry: { mode: from.mode === to.mode ? onto.mode : to.mode, oid: await writeBlobBytes(wt.repo, latin1Bytes(applied.join(''))) } };
}

/** The stash's index change (B → I) onto `entries`; null when it doesn't apply. */
async function indexPatch(wt: WriteWt, entries: IndexEntry[], b: string, i: string): Promise<IndexEntry[] | null> {
  const now = stage0Of(entries);
  let next = [...entries];
  for (const c of await diffTrees(wt.repo, b, i)) {
    const cur = now.get(c.path) ?? null;
    const r = await patchEntry(wt, c.old, c.new, cur && { oid: cur.oid, mode: cur.mode });
    if (r === null) return null;
    next = next.filter((x) => x.path !== c.path);
    if (r.entry) next.push(keepStat(now, { path: c.path, mode: r.entry.mode, oid: r.entry.oid, stage: 0 }));
  }
  return next;
}

/** `untracked`: an untracked file of the stash was in the way (git's "already exists, no checkout",
 *  or a file where its folder goes),
 *  so the stash holds the only copy of it; the autostash's restore (Task 20) checks it first. */
export type StashApplied =
  | { kind: 'applied' }
  | { kind: 'indexRefused' }
  | { kind: 'refused'; error: GbError; untracked?: GbError }
  | { kind: 'conflicts'; paths: string[]; untracked?: GbError }
  | { kind: 'partial'; error: GbError };

const OVERWRITTEN = 'Your local changes to the following files would be overwritten by merge:';

/** The first folder of `path` (outermost first) where a file or link stands instead. */
function blockedFolder(wt: WriteWt, path: string): string | null {
  const parts = path.split('/');
  for (let i = 1; i < parts.length; i++) {
    const q = parts.slice(0, i).join('/');
    const k = kind(wt, `${wt.root}/${q}`);
    if (k === null) return null;
    if (k !== 'dir') return q;
  }
  return null;
}

export async function applyStash(wt: WriteWt, oid: string, withIndex: boolean): Promise<StashApplied> {
  const w = await readCommit(wt.repo, oid);
  const [b, i, u] = w.parents;
  const entries = await loadIndex(wt);
  // 1. git's own words; it prints the `<path>: needs merge` lines on stdout.
  if (unmergedOf(entries).size > 0) throw gitFailure('could not write index', 'error: could not write index\n');
  const head = readHead(wt).oid!;
  const headTree = (await readCommit(wt.repo, head)).tree;
  const ours = await writeIndexTree(wt.repo, entries);
  // 2. The index part (`--index`, when the stash has one) must apply to the index as it is. The
  //    core's `route`: one staged already is no change (the merge then based on it); a staged
  //    part only, onto files edited since, is its patch to the index and the files.
  const iTree = i === undefined ? null : (await readCommit(wt.repo, i)).tree;
  let indexPart = withIndex && i !== undefined && iTree !== (await readCommit(wt.repo, b)).tree;
  let base = b;
  if (indexPart) {
    const part = await diffTrees(wt.repo, b, i);
    const now = stage0Of(entries);
    const holds = (p: string, e: FileEntry | null) => sameEntry(now.get(p) ?? null, e);
    if (part.every((c) => holds(c.path, c.new))) {
      [base, indexPart] = [i, false];
    } else if (part.every((c) => holds(c.path, c.new) || holds(c.path, c.old)) && part.some((c) => holds(c.path, c.new))) {
      // Staged already in some files, not in the others (a stash parts were added to).
      return applyPatch(wt, entries, await stashFiles(wt, b, i, oid), u === undefined ? new Map() : await listFiles(wt.repo, u));
    } else if (iTree === w.tree && u === undefined && part.every((c) => holds(c.path, c.old))) {
      const edited = new Set((await statusEntries(wt.repo, wt.root)).filter((s) => s.kind !== 'untracked' && s.worktree !== '.').map((s) => s.path));
      if (part.some((c) => edited.has(c.path))) return applyPatch(wt, entries, await stashFiles(wt, b, i, oid), new Map());
    }
  }
  const patched = indexPart ? await indexPatch(wt, entries.filter((e) => !e.intentToAdd), b, i) : null;
  if (indexPart && patched === null) return { kind: 'indexRefused' };
  // 3. The untracked files; one where something already is stays out. A file or link standing
  //    where one needs a folder stops git there (`fatal: cannot create directory`): the ones
  //    after it aren't written, and the tracked merge still runs.
  const lines: string[] = [];
  if (u !== undefined) {
    for (const [p, e] of await listFiles(wt.repo, u)) {
      const blocked = blockedFolder(wt, p);
      if (blocked !== null) {
        lines.push(`fatal: cannot create directory at '${blocked}': File exists`);
        break;
      }
      if (wt.fs.existsSync(`${wt.root}/${p}`, false)) lines.push(`${p} already exists, no checkout`);
      else await checkoutEntry(wt, p, e);
    }
  }
  const notes = lines.length === 0 ? '' : `${lines.join('\n')}\nerror: could not restore untracked files from stash\n`;
  const unstashed = withIndex ? 'Index was not unstashed.\n' : '';
  // The core's words: git's first line, its kind from the whole stderr.
  const untracked = lines.length === 0 ? null : gitFailure(lines[0].replace(/^fatal: /, ''), notes);
  const withUntracked = untracked === null ? {} : { untracked };
  // 4. git's merge wants the index to hold ours (the index's own tree). With an index part git
  // resets the index to HEAD first, so a staged change would refuse: the core's `index_aside` runs
  // it on a temp index then, the other files' staged state untouched, and a path of the stash
  // that's staged now refuses the index. An intent-to-add entry always refuses.
  let staged: string[];
  if (indexPart) {
    const changed = (await diffTrees(wt.repo, headTree, ours)).map((c) => c.path);
    if (changed.length === 0) await mixedReset(wt, head);
    else {
      const own = new Set([...(await diffTrees(wt.repo, b, i!)), ...(await diffTrees(wt.repo, b, oid))].map((c) => c.path));
      if (changed.some((p) => own.has(p))) return { kind: 'indexRefused' };
    }
    staged = [];
  } else staged = entries.filter((e) => e.intentToAdd).map((e) => e.path);
  if (staged.length > 0) return { kind: 'refused', error: gitFailure(OVERWRITTEN, `error: ${OVERWRITTEN}\n  ${staged.join(' ')}\n${unstashed}${notes}`), ...withUntracked };
  const attrs = new AttrReader(wt.repo, wt.root);
  const m = await mergeTrees(wt.repo, base, ours, oid, { ours: 'Updated upstream', theirs: 'Stashed changes' }, (p) => markerSizeOf(attrs, p));
  const problems = worktreeProblems(wt, m, await statusEntries(wt.repo, wt.root));
  if (problems.size > 0) {
    const e = refusal('merge', problems, (p) => p);
    return { kind: 'refused', error: gitFailure(e.message, `${e.stderr ?? ''}${unstashed}${notes}`), ...withUntracked };
  }
  // 5. The merge into the worktree and the index.
  await applyMerge(wt, m);
  if (m.conflicts.length > 0) return { kind: 'conflicts', paths: m.conflicts, ...withUntracked };
  const merged = stage0Of(await loadIndex(wt));
  let final: IndexEntry[];
  if (patched !== null) final = patched.map((e) => keepStat(merged, e));
  else {
    const baseFiles = await listFiles(wt.repo, base);
    const before = stage0Of(entries);
    final = entries.map((e) => keepStat(merged, e));
    for (const [p, e] of await listFiles(wt.repo, oid)) if (!baseFiles.has(p) && !before.has(p)) final.push(keepStat(merged, { path: p, mode: e.mode, oid: e.oid, stage: 0 }));
  }
  await saveIndex(wt, final);
  // 6. An untracked file that was in the way.
  if (untracked !== null) return { kind: 'partial', error: untracked };
  return { kind: 'applied' };
}

/** A stash's file: its entries on the base (`b`), in the index part and in the worktree part. */
interface StashFile {
  path: string;
  b: FileEntry | null;
  i: FileEntry | null;
  w: FileEntry | null;
}

async function stashFiles(wt: WriteWt, b: string, i: string, w: string): Promise<StashFile[]> {
  const files = new Map<string, StashFile>();
  for (const c of await diffTrees(wt.repo, b, i)) files.set(c.path, { path: c.path, b: c.old, i: c.new, w: c.old });
  for (const c of await diffTrees(wt.repo, b, w)) files.set(c.path, { ...(files.get(c.path) ?? { path: c.path, b: c.old, i: c.old }), w: c.new });
  return [...files.values()].sort((x, y) => compareBytewise(x.path, y.path));
}

/** `Route::Merge`: each file's index entry to the index part's, and the file the worktree part's
 *  changes on top of the entry it had (the index part's or the base's) by a 3-way merge; the
 *  untracked files written. Every file merged first; nothing changes on a refusal. */
async function applyPatch(wt: WriteWt, entries: IndexEntry[], part: StashFile[], untracked: Map<string, FileEntry>): Promise<StashApplied> {
  const now = stage0Of(entries);
  const files = new Map<string, FileEntry | null>();
  const refuse = (p: string, why: string): StashApplied => ({ kind: 'refused', error: gbError('Conflict', `The stash's changes don't go into ${p} as it is now: ${why}; nothing changed`) });
  let next = [...entries];
  for (const c of part) {
    const cur = now.get(c.path);
    const idx = cur ? { mode: cur.mode, oid: cur.oid } : null;
    const from = sameEntry(idx, c.i) ? c.i : sameEntry(idx, c.b) ? c.b : undefined;
    if (from === undefined) return refuse(c.path, "it's staged otherwise");
    const file = await mergeEntry(wt, await fileEntry(wt, c.path), from, c.w);
    if ('clash' in file) return refuse(c.path, 'it has edits on or next to the same lines');
    files.set(c.path, file.entry);
    if (sameEntry(idx, c.i)) continue;
    next = next.filter((x) => x.path !== c.path);
    if (c.i) next.push(keepStat(now, { path: c.path, mode: c.i.mode, oid: c.i.oid, stage: 0 }));
  }
  for (const [p, e] of untracked) {
    const f = await fileEntry(wt, p);
    if (f === null) files.set(p, e);
    else if (!sameEntry(f, e)) return refuse(p, 'an untracked file is there already');
  }
  for (const [p, e] of files) {
    if (e === null) removeWorktreePath(wt, p);
    else await checkoutEntry(wt, p, e);
  }
  await saveIndex(wt, next);
  return { kind: 'applied' };
}

// --- Add to the latest stash (the core's `combine`, `predicted` and `fold`) ---

/** One side of a stash on its base: each changed file's entries. */
type Side = Map<string, { base: FileEntry | null; now: FileEntry | null }>;

/** A stash's changes on its base: its index part, its worktree part, its untracked files. */
export interface StashParts {
  i: Side;
  w: Side;
  u: Map<string, FileEntry>;
}

export interface StashShape {
  base: string;
  index: string;
  untracked: string | null;
  parts: StashParts;
}

const sideOf = async (wt: WriteWt, a: string, b: string): Promise<Side> => new Map((await diffTrees(wt.repo, a, b)).map((c) => [c.path, { base: c.old, now: c.new }]));

export async function stashShape(wt: WriteWt, oid: string): Promise<StashShape> {
  const [base, index, untracked, ...more] = (await readCommit(wt.repo, oid)).parents;
  if (index === undefined || more.length > 0 || (await readCommit(wt.repo, index)).parents.join(' ') !== base) fail('InvalidInput', "That stash isn't shaped as git makes one; nothing was added");
  const u = untracked === undefined ? new Map<string, FileEntry>() : new Map(await listFiles(wt.repo, untracked));
  return { base, index, untracked: untracked ?? null, parts: { i: await sideOf(wt, base, index), w: await sideOf(wt, base, oid), u } };
}

const WHY: Record<Clash, string> = {
  lines: 'has changes in both on or next to the same lines',
  binary: 'is a binary file changed in both',
  link: 'is a symbolic link changed in both',
  kind: 'is a file in one and a symbolic link in the other',
  submodule: 'is a submodule changed in both',
  whole: 'is added or deleted in one and changed in the other',
};
const addRefusal = (into: string, path: string, why: string) => gbError('Conflict', `Can't add to "${into}": ${path} ${why}. Nothing changed`);

/** Each part's files 3-way merged, the old stash's changes as ours, the new ones as theirs; the
 *  untracked files' union. Throws the refusal for a file whose changes don't combine. */
export async function combineParts(wt: WriteWt, old: StashParts, add: StashParts, into: string): Promise<StashParts> {
  const sides: Side[] = [new Map(old.i), new Map(old.w)];
  for (const [k, [ours, theirs]] of ([[old.i, add.i], [old.w, add.w]] as const).entries()) {
    for (const [p, { base, now }] of theirs) {
      const o = ours.has(p) ? ours.get(p)!.now : base;
      const r = await mergeEntry(wt, o, base, now);
      if ('clash' in r) throw addRefusal(into, p, WHY[r.clash]);
      sides[k].set(p, { base, now: r.entry });
    }
  }
  const tracked = (s: StashParts, p: string) => s.i.has(p) || s.w.has(p);
  const cross = [...old.u.keys()].find((p) => tracked(add, p)) ?? [...add.u.keys()].find((p) => tracked(old, p));
  if (cross !== undefined) throw addRefusal(into, cross, 'is untracked in one and tracked in the other');
  const u = new Map(old.u);
  for (const [p, e] of add.u) {
    const o = old.u.get(p);
    if (o && !sameEntry(o, e)) throw addRefusal(into, p, 'is untracked in both, with different contents');
    u.set(p, e);
  }
  return { i: sides[0], w: sides[1], u };
}

/** What a push of `scope`'s `part` would stash, for the files in `keys` only (the core's
 *  `predicted`): Staged's staged changes as both parts; Unstaged's (`scope` its picked files)
 *  staged changes as the index part, the files as they are as the worktree part, and the
 *  untracked ones. */
export async function predictParts(wt: WriteWt, scope: string[], part: 'staged' | 'unstaged', keys: Set<string>): Promise<StashParts> {
  const headFiles = await listFiles(wt.repo, readHead(wt).oid!);
  const index = stage0Of((await loadIndex(wt)).filter((e) => !e.intentToAdd));
  const out: StashParts = { i: new Map(), w: new Map(), u: new Map() };
  for (const p of [...keys].filter(inScope(scope))) {
    const base = headFiles.get(p) ?? null;
    const at = index.get(p);
    const staged = at ? { mode: at.mode, oid: at.oid } : null;
    if (!sameEntry(base, staged)) out.i.set(p, { base, now: staged });
    if (part === 'staged') {
      if (!sameEntry(base, staged)) out.w.set(p, { base, now: staged });
      continue;
    }
    const file = await fileEntry(wt, p);
    if (base === null && staged === null) {
      if (file) out.u.set(p, file);
    } else if (!sameEntry(base, file)) out.w.set(p, { base, now: file });
  }
  return out;
}

/** git's `commit-tree` of `tree` with `of`'s message, author and committer. */
async function commitLike(wt: WriteWt, of: string, tree: string, parents: string[]): Promise<string> {
  const raw = await readRaw(wt.repo, of);
  const h = readCommitHeaders(raw!.bytes);
  const text = (b: Uint8Array) => new TextDecoder().decode(b);
  return writeCommitObject(wt.repo, { tree, parents, author: text(h.author), committer: text(h.committer), message: text(h.messageBytes) });
}

/** The stash `add` folded into `old` (the stash `oid`, on the same base): W, I and U as git
 *  shapes them, with `oid`'s message, identity and dates, stored on top as `subject`. */
export async function foldStash(wt: WriteWt, ident: string, old: StashShape, oid: string, add: string, subject: string, into: string): Promise<string> {
  const n = await stashShape(wt, add);
  if (n.base !== old.base) fail('Stale', 'HEAD moved during the stash');
  const c = await combineParts(wt, old.parts, n.parts, into);
  const treeOf = async (side: Side) => {
    const files = new Map(await listFiles(wt.repo, old.base));
    for (const [p, { now }] of side) {
      if (now) files.set(p, now);
      else files.delete(p);
    }
    return writeTreeOf(wt.repo, [...files].map(([path, e]) => ({ path, mode: e.mode, oid: e.oid })));
  };
  const i = await commitLike(wt, old.index, await treeOf(c.i), [old.base]);
  let untracked = old.untracked ?? n.untracked;
  if (old.untracked !== null && n.untracked !== null && c.u.size > old.parts.u.size) {
    untracked = await commitLike(wt, old.untracked, await writeTreeOf(wt.repo, [...c.u].map(([path, e]) => ({ path, mode: e.mode, oid: e.oid }))), []);
  }
  const w = await commitLike(wt, oid, await treeOf(c.w), [old.base, i, ...(untracked === null ? [] : [untracked])]);
  storeStash(wt, ident, w, subject);
  return w;
}

// The safety review's C1 and C2 (precheck.rs): a stash that writes a file where a folder holding a
// repository stands would delete that repository whole, so it's refused first.

/** precheck.rs's `repository_in_the_way`: `what` is "stash", "apply" or "pop". */
export const repositoryInTheWay = (path: string, what: string): GbError => gbError('InvalidInput', `${path} is a repository in the way of the ${what}: move it first`);

/** precheck.rs's `repos_at`: the repositories a file written at each of `files` would delete (a `.git`
 *  in the real folder there, or a populated gitlink at or under it), sorted. */
function reposAt(wt: WriteWt, links: Iterable<string>, files: Iterable<string>): string[] {
  const out = new Set<string>();
  const gitlinks = [...links];
  for (const p of files) {
    if (throughLink(wt, p) || kind(wt, `${wt.root}/${p}`) !== 'dir') continue;
    const r = repoWithin(wt, p);
    if (r !== null) {
      out.add(r);
      continue;
    }
    for (const g of gitlinks) {
      if (g !== p && !g.startsWith(`${p}/`)) continue;
      if (!throughLink(wt, g) && kind(wt, `${wt.root}/${g}`) === 'dir' && wt.fs.readdirSync(`${wt.root}/${g}`).length > 0) out.add(g);
    }
  }
  return [...out].sort(compareBytewise);
}

const indexGitlinks = async (wt: WriteWt) => (await loadIndex(wt)).filter((e) => e.mode === MODE_GITLINK).map((e) => e.path);
const isFile = (e: FileEntry | null) => e !== null && e.mode !== MODE_GITLINK;

/** C1 (`stash_push_in_the_way`): the stash's own reset writes HEAD's file at each dirty tracked path
 *  (within `pathspec`) over whatever stands there. */
export async function pushInTheWay(wt: WriteWt, tracked: Set<string>, pathspec: string[] | null): Promise<string[]> {
  const head = readHead(wt).oid;
  if (tracked.size === 0 || head === null) return [];
  const files = await listFiles(wt.repo, head);
  const scope = inScope(pathspec);
  return reposAt(wt, await indexGitlinks(wt), [...tracked].filter((p) => scope(p) && isFile(files.get(p) ?? null)));
}

/** C2 (`stash_apply_in_the_way`): the apply writes the stash's files (W's and I's against its base,
 *  and its untracked ones) over whatever stands there. */
export async function applyInTheWay(wt: WriteWt, oid: string): Promise<string[]> {
  const [b, i, u] = (await readCommit(wt.repo, oid)).parents;
  if (b === undefined) return [];
  const links = await indexGitlinks(wt);
  const files: string[] = [];
  const take = (changes: TreeChange[]) => {
    for (const c of changes) {
      if (isFile(c.new)) files.push(c.path);
      if (c.old?.mode === MODE_GITLINK) links.push(c.path);
    }
  };
  take(await diffTrees(wt.repo, b, oid));
  if (i !== undefined) take(await diffTrees(wt.repo, b, i));
  if (u !== undefined) for (const [p, e] of await listFiles(wt.repo, u)) if (isFile(e)) files.push(p);
  return reposAt(wt, links, files);
}

export async function stashPaths(wt: WriteWt, oid: string): Promise<string[]> {
  const [b, i, u] = (await readCommit(wt.repo, oid)).parents;
  const out = new Set<string>();
  for (const c of await diffTrees(wt.repo, b, oid)) out.add(c.path);
  if (i !== undefined) for (const c of await diffTrees(wt.repo, b, i)) out.add(c.path);
  if (u !== undefined) for (const p of (await listFiles(wt.repo, u)).keys()) out.add(p);
  return [...out].sort(compareBytewise);
}

export async function binaryAmong(wt: WriteWt, oid: string, paths: string[]): Promise<boolean> {
  const b = (await readCommit(wt.repo, oid)).parents[0];
  const [before, after] = [await listFiles(wt.repo, b), await listFiles(wt.repo, oid)];
  for (const p of paths) {
    for (const e of [before.get(p), after.get(p)]) if (e && e.mode !== MODE_GITLINK && isBinaryHead(await readBlobBytes(wt.repo, e.oid))) return true;
  }
  return false;
}
