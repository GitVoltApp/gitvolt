// The app build's UI step (tauri.conf.json's beforeBuildCommand): `npm run build` (tsc, vite build
// into dist, the entry-chunk check), unless dist was built from the same inputs and hasn't changed
// since. The inputs: every file under ui/ but the build outputs and node_modules (whose contents
// the lockfiles stand for), the files from outside ui/ the build reads (about.toml's allow-list,
// the notices it copies in from target/licenses, tauri.conf.json), the VITE_* environment and
// Node's version. A `just package` after a Rust-only change then skips the UI's 1-2 minutes, and
// the license check of the UI's packages still reruns whenever the lockfile or the allow-list
// changes. GITVOLT_REBUILD=1 always builds (docs/dev-setup.md, "Packaging").
import { execSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const ui = join(dirname(fileURLToPath(import.meta.url)), '..');
const root = join(ui, '..');
const dist = join(ui, 'dist');
// Outside dist, which the app embeds as it is.
const stamp = join(root, 'target', 'ui-build', 'dist.key');
const licensesDir = process.env.GITVOLT_LICENSES_DIR ?? join(root, 'target', 'licenses');
// Build outputs and per-run files, at any depth under ui/.
const skip = new Set(['node_modules', 'dist', 'dist-e2e', 'dist-demo-e2e', 'test-results', 'playwright-report', '.fixtures.json']);

function* files(dir) {
  for (const name of readdirSync(dir).sort()) {
    if (skip.has(name)) continue;
    const path = join(dir, name);
    if (statSync(path).isDirectory()) yield* files(path);
    else yield path;
  }
}

function digest(paths) {
  const hash = createHash('sha256');
  for (const path of paths) {
    hash.update(`${relative(root, path)}\n`);
    hash.update(existsSync(path) ? readFileSync(path) : '(missing)');
    hash.update('\n');
  }
  return hash.digest('hex');
}

function distDigest() {
  if (!existsSync(join(dist, 'index.html'))) return '';
  const all = [];
  const walk = (dir) => {
    for (const name of readdirSync(dir).sort()) {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) walk(path);
      else all.push(path);
    }
  };
  walk(dist);
  return digest(all);
}

const env = Object.keys(process.env).filter((k) => k.startsWith('VITE_') || k === 'NODE_ENV').sort().map((k) => `${k}=${process.env[k]}`);
const inputs = createHash('sha256')
  .update(`node=${process.version}\n${env.join('\n')}\n`)
  .update(digest([
    ...files(ui),
    join(ui, 'node_modules', '.package-lock.json'),
    join(root, 'about.toml'),
    join(root, 'LICENSE'),
    join(root, 'crates', 'gitvolt-app', 'dictionaries', 'en-US-LICENSE.txt'),
    join(root, 'crates', 'gitvolt-app', 'tauri.conf.json'),
    join(licensesDir, 'THIRD-PARTY-NOTICES-rust.txt'),
    join(licensesDir, 'CEF-LICENSE.txt'),
  ]))
  .digest('hex');

const [had, hadDist] = existsSync(stamp) ? readFileSync(stamp, 'utf8').split('\n') : [];
if (process.env.GITVOLT_REBUILD !== '1' && had === inputs && hadDist && hadDist === distDigest()) {
  console.log('UI build is up to date (ui/dist; GITVOLT_REBUILD=1 rebuilds it)');
} else {
  execSync('npm run build', { cwd: ui, stdio: 'inherit' });
  mkdirSync(dirname(stamp), { recursive: true });
  writeFileSync(stamp, `${inputs}\n${distDigest()}\n`);
}
