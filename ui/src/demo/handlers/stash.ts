// Stashes (write/stash.rs): identity is the stash commit's id, never a list position.
// Push and pop and drop are journaled (UndoKind::Stash); apply isn't (the stash is kept).
import type { GbError } from '../../api/gen/GbError';
import type { StashPart } from '../../api/gen/StashPart';
import type { StashApplyOutcome } from '../../api/gen/StashApplyOutcome';
import type { StashPushOutcome } from '../../api/gen/StashPushOutcome';
import type { HandlerTable } from '../transport';
import { gbError } from '../engine/errors';
import { readCommit } from '../engine/read/commitText';
import { fail } from '../engine/read/seam';
import type { JournalEntry, StashMove } from '../engine/write/journal';
import { readHead } from '../engine/write/refs';
import { runWrite, type WriteCx, type WriteIntent } from '../engine/write/run';
import { writeTarget } from '../engine/write/seam';
import { createSnapshot, restoreSnapshot, untrackedAmong } from '../engine/write/snapshot';
import { applyInTheWay, applyStash, combineParts, dropStash, foldStash, predictParts, pushInTheWay, pushStaged, pushStash, repositoryInTheWay, stashList, stashPaths, stashShape, storeStash, subjectOf, userMessage, type StashApplied } from '../engine/write/stash';
import { dirty, statusEntries } from '../engine/write/status';
import type { WriteWt } from '../engine/write/seam';
import { NO_EXPECT } from './stage';

export function applyError(r: StashApplied): GbError | null {
  switch (r.kind) {
    case 'applied':
    case 'conflicts':
      return null;
    case 'indexRefused':
      return { ...gbError('Conflict', "git couldn't restore what was staged"), detail: { kind: 'applyWithoutIndex' } };
    default:
      return r.error;
  }
}

export function forgetKept(cx: WriteCx, oid: string): void {
  if (cx.journal.forgetKeptStash(oid)) cx.journalChanged = true;
}


const firstLine = (m: string) => (m.split('\n')[0] ?? '').trim();

/** Why a pop whose `after` snapshot failed can't be undone (stash.rs's NO_AFTER). */
const NO_AFTER = "Can't be undone: GitVolt couldn't record what the pop left";

/** The core's `relist` lists a multi-line message again by its first line (a drop, then a store),
 *  guarded by a pending kept-stash record that goes once the stash is listed again. The demo's
 *  store is atomic, so the guard shows only as the journal id it takes, as the core's does. */
function relistGuard(cx: WriteCx, oid: string, subject: string, message: string, label: string): void {
  if (!message.trimEnd().includes('\n')) return;
  // The core's early return: a stored line that already reads as the summary needs no relist.
  // The demo keeps the summary as the subject, so git's folded line is rebuilt from the message.
  const prefix = subject.slice(0, subject.length - userMessage(subject).length);
  const folded = `${prefix}${message.trim().split('\n').map((l) => l.trim()).filter(Boolean).join(' ')}`;
  if (folded === `${prefix}${firstLine(message)}`) return;
  const id = cx.journal.keep({ oid, message: firstLine(message), label, target: null, reason: { status: 'pending' }, createdMs: cx.ctx.clock.nowMs() });
  cx.journal.kept = cx.journal.kept.filter((k) => k.id !== id);
  cx.journalChanged = true;
}

/** The safety review's C1 and C2: the first repository a stash's write would delete refuses it. */
function refuseInTheWay(found: string[], what: string): void {
  if (found.length > 0) throw repositoryInTheWay(found[0], what);
}

const within = (p: string, scope: string[]) => scope.some((s) => p === s || p.startsWith(`${s}/`));

/** The core's `with_renames`: `paths` and the other side of each staged rename with a side in them. */
async function withRenames(wt: WriteWt, paths: string[]): Promise<string[]> {
  const out = [...paths];
  for (const e of await statusEntries(wt.repo, wt.root)) {
    if (e.origPath === null || !(within(e.path, paths) || within(e.origPath, paths))) continue;
    for (const p of [e.origPath, e.path]) if (!within(p, out)) out.push(p);
  }
  return out;
}

/** Unstaged's files: those with unstaged changes, and the untracked ones (the core's `unstaged_in`). */
async function picked(wt: WriteWt, scope: string[]): Promise<string[]> {
  return (await statusEntries(wt.repo, wt.root)).filter((e) => within(e.path, scope) && e.worktree !== '.').map((e) => e.path);
}

/** `git stash push`; with `paths`, a WIP file's or folder's Stash, narrowed to them, and with
 *  `part` that part of their changes only (the core's `push`). */
async function pushSome(cx: WriteCx, wt: WriteWt, message: string, paths: string[] | null, part: StashPart | null = null) {
  const scope = paths === null ? null : await withRenames(wt, paths);
  if (scope === null || part === null) return pushStash(wt, cx.ident(), message, { untracked: 'untracked', paths: scope, narrow: scope !== null });
  if (part === 'staged') return pushStaged(wt, cx.ident(), message, scope);
  const files = await picked(wt, scope);
  if (files.length === 0) return null;
  return pushStash(wt, cx.ident(), message, { untracked: 'untracked', paths: await withRenames(wt, files), narrow: true, keepStaged: true });
}

/** The core's `add`: `paths`' `part` stashed (`pushSome`), then folded into `into` (stash@{0});
 *  a file whose changes don't combine refuses first, nothing changed. `entry`: a redo's, whose
 *  moves it replaces. The combined stash, or null when there was nothing to stash. */
async function addTo(cx: WriteCx, into: string, subject: string, message: string, paths: string[], part: StashPart, entry: number | null): Promise<string | null> {
  const wt = cx.wt;
  const name = userMessage(subject);
  if (stashList(wt)[0]?.oid !== into) fail('Stale', 'The latest stash changed; nothing was added');
  const old = await stashShape(wt, into);
  if (readHead(wt).oid !== old.base) fail('InvalidInput', `"${name}" was stashed on another commit (HEAD moved since): stash the changes on their own instead. Nothing changed`);
  const keys = new Set([...old.parts.i.keys(), ...old.parts.w.keys(), ...old.parts.u.keys()]);
  if (keys.size > 0) {
    const scope = await withRenames(wt, paths);
    await combineParts(wt, old.parts, await predictParts(wt, part === 'staged' ? scope : await withRenames(wt, await picked(wt, scope)), part, keys), name);
  }
  const s = await pushSome(cx, wt, message, paths, part);
  cx.command('stash', ...(part === 'staged' ? ['create', message] : ['push', '--include-untracked', '-m', message, '--', ...paths]));
  if (s === null) return null;
  for (const k of ['stash', 'worktree', 'index'] as const) cx.touch(k);
  const record = (moves: StashMove[]) => {
    if (entry === null) for (const m of moves) cx.recordStash(m);
    else edit(cx, entry, (e) => {
      e.stashes = moves;
    });
  };
  let combined: string;
  try {
    combined = await foldStash(wt, cx.ident(), old, into, s.oid, subject, name);
  } catch (e) {
    // The changes are in a stash of their own, kept: Undo applies it, as a push's.
    record([{ oid: s.oid, message: s.subject, created: true, withoutIndex: false, paths, part }]);
    const why = (e as GbError).message;
    throw { ...(e as GbError), message: `The changes are stashed on their own as "${userMessage(s.subject)}": they couldn't be added to "${name}" (${why})` };
  }
  cx.command('stash', 'store', '-m', subject, combined);
  record([
    { oid: combined, message: subject, created: true, withoutIndex: false, paths, part, added: s.oid },
    { oid: into, message: subject, created: false, withoutIndex: false },
  ]);
  dropStash(wt, s.oid);
  dropStash(wt, into);
  forgetKept(cx, into);
  return combined;
}

function add(into: string, subject: string, message: string, paths: string[], part: StashPart): WriteIntent<StashPushOutcome> {
  return {
    kind: 'stash',
    label: () => `add to stash "${userMessage(subject)}"`,
    undo: () => 'stash',
    plan: async (pre) => {
      if (paths.length === 0) fail('InvalidInput', 'No files to stash');
      if (stashList(pre.wt)[0]?.oid !== into) fail('Stale', 'The latest stash changed; nothing was added');
      refuseInTheWay(await pushInTheWay(pre.wt, (await dirty(pre.wt.repo, pre.wt.root)).tracked, await withRenames(pre.wt, paths)), 'stash');
      return {};
    },
    run: async (cx) => {
      const oid = await addTo(cx, into, subject, message, paths, part, null);
      return oid === null ? { status: 'nothingToStash' } : { status: 'stashed', oid };
    },
  };
}

function push(message: string, paths: string[] | null, part: StashPart | null): WriteIntent<StashPushOutcome> {
  return {
    kind: 'stash',
    label: () => `stash "${firstLine(message)}"`,
    undo: () => 'stash',
    plan: async (pre) => {
      if (paths?.length === 0) fail('InvalidInput', 'No files to stash');
      if (part !== null && paths === null) fail('InvalidInput', "Only some files' staged or unstaged changes can be stashed");
      const scope = paths === null ? null : await withRenames(pre.wt, paths);
      refuseInTheWay(await pushInTheWay(pre.wt, (await dirty(pre.wt.repo, pre.wt.root)).tracked, scope), 'stash');
      return {};
    },
    run: async (cx) => {
      const s = await pushSome(cx, cx.wt, message, paths, part);
      cx.command('stash', ...(part === 'staged' ? ['create', message] : ['push', '--include-untracked', '-m', message, ...(paths === null ? [] : ['--', ...paths])]));
      if (s === null) return { status: 'nothingToStash' };
      for (const k of ['stash', 'worktree', 'index'] as const) cx.touch(k);
      cx.recordStash({ oid: s.oid, message: s.subject, created: true, withoutIndex: false, ...(paths && { paths }), ...(part && { part }) });
      relistGuard(cx, s.oid, s.subject, message, `stash "${firstLine(message)}"`);
      return { status: 'stashed', oid: s.oid };
    },
  };
}

function apply(oid: string, pop: boolean, withoutIndex: boolean, subject: string): WriteIntent<StashApplyOutcome> {
  const label = `${pop ? 'pop' : 'apply'} stash "${userMessage(subject)}"`;
  return {
    kind: 'stash',
    label: () => label,
    undo: () => (pop ? 'stash' : null),
    plan: async (pre) => {
      subjectOf(pre.wt, oid);
      refuseInTheWay(await applyInTheWay(pre.wt, oid), pop ? 'pop' : 'apply');
      if (!pop) return {};
      const paths = await stashPaths(pre.wt, oid);
      return { snapshot: { paths, untracked: await untrackedAmong(pre.wt, paths) } };
    },
    run: async (cx) => {
      cx.command('stash', 'apply', ...(withoutIndex ? [] : ['--index']), oid);
      cx.touch('worktree');
      cx.touch('index');
      const r = await applyStash(cx.wt, oid, !withoutIndex);
      const error = applyError(r);
      if (r.kind !== 'indexRefused') cx.partial = true;
      if (error) throw error;
      if (r.kind === 'conflicts') return { status: 'conflicts', files: r.paths.length };
      if (pop) {
        cx.recordStash({ oid, message: subject, created: false, withoutIndex });
        dropStash(cx.wt, oid);
        forgetKept(cx, oid);
        // The pop is done: a failed `after` only costs its undo.
        try {
          cx.after = await createSnapshot(cx.wt, cx.snapshot?.paths ?? [], []);
        } catch {
          cx.editEntry((e) => {
            e.blocked = NO_AFTER;
          });
        }
      }
      return { status: 'applied' };
    },
  };
}

function drop(oid: string, subject: string): WriteIntent<null> {
  return {
    kind: 'stash',
    label: () => `drop stash "${userMessage(subject)}"`,
    undo: () => 'stash',
    run: async (cx) => {
      // Nothing changes when the stash is gone (NotFound), so no entry: the core unrecords it too.
      dropStash(cx.wt, oid);
      cx.recordStash({ oid, message: subject, created: false, withoutIndex: false });
      forgetKept(cx, oid);
      cx.command('stash', 'drop');
      return null;
    },
  };
}

/** Edits the entry an undo or redo acts on (the undo itself has no entry). */
function edit(cx: WriteCx, id: number, f: (e: JournalEntry) => void): void {
  const e = cx.journal.entry(id);
  if (e) f(e);
  cx.journalChanged = true;
}

const conflictKept = () => gbError('Conflict', "The stash's changes conflict; it was kept");

export async function undoStash(cx: WriteCx, entry: JournalEntry, undo: boolean, label: string, withoutIndex: boolean): Promise<void> {
  const wt = cx.wt;
  const moved = entry.stashes[0] ?? null;
  // An add: its own stash applied and the replaced one stored back; redo adds again.
  if (moved?.created && moved.added) {
    const replaced = entry.stashes[1];
    if (!replaced || replaced.created) fail('Other', "the replaced stash isn't recorded");
    if (undo) {
      subjectOf(wt, moved.oid);
      cx.touch('worktree');
      cx.touch('index');
      const r = await applyStash(wt, moved.added, !withoutIndex);
      const error = applyError(r);
      if (error) throw error;
      if (r.kind === 'conflicts') throw conflictKept();
      if (!stashList(wt).some((s) => s.oid === replaced.oid)) storeStash(wt, cx.ident(), replaced.oid, replaced.message);
      dropStash(wt, moved.oid);
      cx.touch('stash');
      return;
    }
    const full = (await readCommit(wt.repo, moved.added)).rawMessage.trimEnd();
    if ((await addTo(cx, replaced.oid, replaced.message, userMessage(full), moved.paths ?? [], moved.part ?? 'staged', entry.id)) === null) fail('InvalidInput', 'Nothing to stash');
    return;
  }
  if (moved?.created) {
    if (undo) {
      subjectOf(wt, moved.oid);
      cx.touch('worktree');
      cx.touch('index');
      const r = await applyStash(wt, moved.oid, !withoutIndex);
      const error = applyError(r);
      if (error) throw error;
      if (r.kind === 'conflicts') throw conflictKept();
      dropStash(wt, moved.oid);
      cx.touch('stash');
      return;
    }
    const full = (await readCommit(wt.repo, moved.oid)).rawMessage.trimEnd();
    const message = userMessage(full);
    const paths = moved.paths ?? null;
    const part = moved.part ?? null;
    const s = await pushSome(cx, wt, message, paths, part);
    if (s === null) fail('InvalidInput', 'Nothing to stash');
    cx.touch('stash');
    cx.touch('worktree');
    cx.touch('index');
    edit(cx, entry.id, (e) => {
      e.stashes[0] = { oid: s.oid, message: s.subject, created: true, withoutIndex: false, ...(paths && { paths }), ...(part && { part }) };
    });
    relistGuard(cx, s.oid, s.subject, message, label);
    return;
  }
  if (entry.before !== null) {
    cx.touch('worktree');
    cx.touch('index');
    if (undo) {
      if (moved && !stashList(wt).some((s) => s.oid === moved.oid)) {
        storeStash(wt, cx.ident(), moved.oid, moved.message);
        cx.touch('stash');
      }
      await restoreSnapshot(wt, entry.before);
      return;
    }
    if (!moved) fail('InvalidInput', 'That pop kept its stash; nothing to redo');
    const without = withoutIndex || moved.withoutIndex;
    subjectOf(wt, moved.oid);
    const paths = await stashPaths(wt, moved.oid);
    const snap = await createSnapshot(wt, paths, await untrackedAmong(wt, paths));
    const r = await applyStash(wt, moved.oid, !without);
    const error = applyError(r);
    if (error) throw error;
    if (r.kind === 'conflicts') {
      edit(cx, entry.id, (e) => {
        e.before = snap;
      });
      throw conflictKept();
    }
    dropStash(wt, moved.oid);
    cx.touch('stash');
    const after = await createSnapshot(wt, snap.paths, []).catch(() => null);
    edit(cx, entry.id, (e) => {
      e.before = snap;
      e.after = after;
      e.blocked = after === null ? NO_AFTER : null;
      if (e.stashes[0]) e.stashes[0].withoutIndex = without;
    });
    return;
  }
  if (moved) {
    if (undo) storeStash(wt, cx.ident(), moved.oid, moved.message);
    else dropStash(wt, moved.oid);
    cx.touch('stash');
  }
}

export const stashHandlers: Pick<HandlerTable, 'stashPush' | 'stashApply' | 'stashDrop'> = {
  stashPush: async ({ repo, worktree, message, paths, part, into }, ctx) => {
    const wt = await writeTarget(ctx, repo, worktree);
    const branch = readHead(wt).branch;
    const on = `WIP on ${branch ?? '(no branch)'}`;
    const named = paths && paths.length > 0 ? `${on}: ${paths[0]}${paths.length > 1 ? ` and ${paths.length - 1} more` : ''}${part ? ` (${part})` : ''}` : on;
    const m = message.trim() === '' ? named : message;
    if (!into) return runWrite(ctx, repo, worktree, NO_EXPECT, push(m, paths ?? null, part ?? null));
    if (!paths || !part) fail('InvalidInput', "Only some files' staged or unstaged changes can be added to a stash");
    return runWrite(ctx, repo, worktree, NO_EXPECT, add(into, subjectOf(wt, into), m, paths, part));
  },
  stashApply: async ({ repo, worktree, oid, pop, withoutIndex }, ctx) => {
    const subject = subjectOf(await writeTarget(ctx, repo, worktree), oid);
    return runWrite(ctx, repo, worktree, NO_EXPECT, apply(oid, pop, withoutIndex === true, subject));
  },
  stashDrop: async ({ repo, worktree, oid }, ctx) => {
    const subject = subjectOf(await writeTarget(ctx, repo, worktree), oid);
    return runWrite(ctx, repo, worktree, NO_EXPECT, drop(oid, subject));
  },
};
