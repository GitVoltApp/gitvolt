import { Tag, Trash2, Upload } from 'lucide-react';
import { shortSha } from '../format/sha';
import type { CommitTarget, MenuEnv, SidebarTarget, TagTarget } from '../menu/menuEnv';
import { registerMenu } from '../menu/registry';
import type { MenuRow, Variant } from '../menu/types';
import { createTagAt } from './create';
import { deleteTag } from './delete';
import { pushTags } from './push';
import { tagOn } from './sources';

/** The remote a tag's Delete ▸ Remote targets (Deviation 9): origin, else the first, of the
 * remotes known to have the tag or not known either way. */
const deleteRemote = (t: TagTarget, names: readonly string[]) => {
  const may = names.filter((r) => tagOn(t.remotes, r) !== 'missing');
  return may.includes('origin') ? 'origin' : may[0] ?? null;
};

/** The remotes a Push of the tag is offered for: all but those known to have it at this commit. */
const pushRemotes = (t: TagTarget, names: readonly string[]) => names.filter((r) => tagOn(t.remotes, r) !== 'same');

/** A Push row's tooltip: a remote whose tag of that name is another commit refuses it (no force). */
function pushTip(t: TagTarget, r: string): string {
  if (tagOn(t.remotes, r) !== 'other') return `Send the tag ${t.name} to ${r}`;
  const at = t.remotes?.find((x) => x.remote === r)?.commit;
  return `${r} has another ${t.name}${at ? ` (at ${shortSha(at)})` : ''}: it refuses the push`;
}

export const offTagMenus = [
  registerMenu<CommitTarget, MenuEnv>({
    id: 'tag.createHere', kind: 'commit', group: 'commit', order: 20,
    when: (t, env) => !t.isWip && !t.isStash && !!env.write,
    rows: (t, env) => {
      const go = (annotated: boolean) => () => { void createTagAt(env.write!, t.sha, annotated); };
      return [{
        kind: 'action', id: 'tag.createHere', label: 'Create tag here', icon: Tag, tooltip: `Create a lightweight tag at ${shortSha(t.sha)}`, run: go(false),
        variants: [
          { id: 'lightweight', label: 'Lightweight', tooltip: `A tag that only names ${shortSha(t.sha)}`, run: go(false) },
          { id: 'annotated', label: 'Annotated', tooltip: 'A tag with its own message, author and date', run: go(true) },
        ],
      }];
    },
  }),
  registerMenu<TagTarget, MenuEnv>({
    id: 'tag.push', kind: 'tag', group: 'sync', order: 0,
    when: (t, env) => !!env.write && pushRemotes(t, env.remoteNames ?? []).length > 0,
    rows: (t, env) => {
      const names = pushRemotes(t, env.remoteNames ?? []);
      const push = (r: string) => () => { void pushTags(env.write!, r, t.name); };
      if (names.length === 1) return [{ kind: 'action', id: 'tag.push', label: `Push ${t.name} to ${names[0]}`, icon: Upload, tooltip: pushTip(t, names[0]), run: push(names[0]) }];
      return [{
        kind: 'submenu', id: 'tag.push', label: `Push ${t.name} to`, icon: Upload, tooltip: `Send the tag ${t.name} to a remote`,
        rows: names.map((r): MenuRow => ({ kind: 'action', id: `tag.push.${r}`, label: r, icon: Upload, tooltip: pushTip(t, r), run: push(r) })),
      }];
    },
  }),
  registerMenu<TagTarget, MenuEnv>({
    id: 'tag.delete', kind: 'tag', group: 'manage', order: 0,
    when: (_, env) => !!env.write,
    rows: (t, env) => {
      const remote = deleteRemote(t, env.remoteNames ?? []);
      const go = (local: boolean, r: string | null) => () => { void deleteTag(env.write!, { tag: t.name, local, remote: r }); };
      const variants: Variant[] = [
        { id: 'local', label: 'Local', tooltip: `Delete the tag ${t.name} here (you can undo this)`, run: go(true, null) },
        ...(remote ? [
          { id: 'remote', label: 'Remote', tooltip: `Delete ${t.name} from ${remote} (a push: can't be undone)`, run: go(false, remote) },
          { id: 'both', label: 'Both', tooltip: `Delete ${t.name} from ${remote}, then here`, run: go(true, remote) },
        ] : []),
      ];
      return [{ kind: 'action', id: 'tag.delete', label: 'Delete', icon: Trash2, tooltip: `Delete the tag ${t.name} here`, run: go(true, null), variants, defaultVariant: 'local' }];
    },
  }),
  registerMenu<SidebarTarget, MenuEnv>({
    id: 'remote.pushTags', kind: 'sidebar', group: 'sync', order: 0,
    when: (t, env) => t.what === 'remote' && !!env.write,
    rows: (t, env) => (t.what === 'remote'
      ? [{ kind: 'action', id: 'remote.pushTags', label: `Push all tags to ${t.name}`, icon: Upload, tooltip: `Send every local tag ${t.name} doesn't have`, run: () => { void pushTags(env.write!, t.name, null); } }]
      : []),
  }),
];
