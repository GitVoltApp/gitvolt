// What's new (src/updates/WhatsNew.tsx): CHANGELOG.md's released sections, read at build time and
// served as `virtual:gitvolt-changelog`, a JSON module of `ChangelogEntry[]` (file order, no
// [Unreleased]). The app never fetches its notes: they're the ones it was built with.
import { readFileSync } from 'node:fs';
import type { Plugin } from 'vite';
import { parseChangelog } from '../src/updates/changelog.ts';

const ID = 'virtual:gitvolt-changelog';

export function changelog(path: string): Plugin {
  return {
    name: 'gitvolt-changelog',
    resolveId: (source) => (source === ID ? `\0${ID}` : null),
    load(id) {
      if (id !== `\0${ID}`) return null;
      this.addWatchFile(path);
      return `export default ${JSON.stringify(parseChangelog(readFileSync(path, 'utf8')))};\n`;
    },
  };
}
