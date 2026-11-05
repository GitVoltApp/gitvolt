# gitvolt-gpui: working on it

The goal: **the web UI (`ui/`, on CEF) rewritten on GPUI, looking and clicking the same.** Same
sizes, colours, wording, shortcuts, menus, hover and arming behaviour. A small difference is fine if
it's named in `docs/decisions/gpui-rewrite.md`. `docs/decisions/gpui-rewrite.md` says what's done and
what's missing; keep it true. Read `AGENTS.md` first: its rules apply here, the UI conventions most.

## How to port something

1. **Read the feature in `ui/src`** (its `.tsx`, `.ts` and `.css`, and its tests, which say what it
   must do) and in `ARCHITECTURE.md`. The code is the spec; port behaviour, not just looks.
2. **See it running**: `tools/webref.mjs` drives the CEF build and takes screenshots and metrics.
3. **Write it** in its own module (`src/<feature>.rs` or `src/<feature>/`); keep `app.rs` small. Pure
   logic goes in functions with unit tests, as `details.rs` and `diff/model.rs` do.
4. **Compare** with `tools/shot.sh` beside the reference, and fix what differs. Read both images.
5. **Commit** when a piece works, one commit per feature.

## The rules that keep this safe

- **Never push.** Commit on the current branch only. No `Co-Authored-By`, no mention of Claude or any
  AI in commits. `git add <paths>`, never `-A`. Commit messages as in `CONTRIBUTING.md`.
- **Builds go on disk, one at a time, with `-j 8`:**
  `cargo build --release -j 8 -p gitvolt-gpui --target-dir target/bench`. Never put a target dir or
  anything large under `/tmp`: it's a RAM disk, and filling it has killed the user's desktop app.
- **Memory is shared with the user's own programs and has run out twice.** Close the CEF reference as
  soon as you have its images, and don't run it during a build. Delete any `core.*` file a crash leaves.
- **Don't touch the user's things.** A GitVolt from `/usr/bin` runs on this machine: never kill or
  drive it (`pkill` only by the exact path you started). No screenshots of the desktop or of "the
  active window"; `tools/shot.sh` finds the window by its own pid. Never run a write against a real
  repository: use throwaway fixtures under `target/bench/work/` (`tools/fixture.sh`), with fictional
  names and emails. Never the real keyring or a real forge: tests use the harness's fake forge.
- **When you run the build**, isolate it: `XDG_CONFIG_HOME`, `XDG_DATA_HOME` and `XDG_CACHE_HOME`
  under `target/bench/work/`, so the user's settings are never read or written.

## How it's put together

- **State:** `GitVolt` (`app.rs`) holds the tabs, and `render` draws everything from it. Sub-views are
  functions taking `&mut Context<GitVolt>`. A diff lives in `Tab::diff` (`diff/view.rs`).
- **The core:** call `Api` from `self.rt.spawn(..)`, await the handle in `cx.spawn`, then
  `this.update`. `api.dispatch(Request::X { .. })` gives JSON (camelCase, as `ui/src/api/gen/` types
  it). Writes go through the same `Request`s, so the core's write pipeline and journal apply. Add a
  typed method to `Api` when a payload is big, as `Api::graph` does.
- **Drawing:** `theme.rs` has `ui/src/theme/tokens.css`'s colours: add tokens there with the CSS's
  name, never a stray hex. Paint-heavy views use `canvas`, as `graph.rs` and `diff/view.rs`.
- **Icons:** add a Lucide name to `ICONS` in `assets/generate.mjs`, run it from `ui/` (`node
  ../crates/gitvolt-gpui/assets/generate.mjs`), then `icon("name", size, color)`.
- **Text:** the UI font is `system-ui`'s; mono text is `diff::view::FONT`. There's no `letter-spacing`
  in GPUI; uppercase captions go without it.
- **No input device here:** `GITVOLT_SHOW` (`bench.rs`) opens things by script for screenshots; extend
  it as needed. Clicks and keys are checked in tests with GPUI's `TestAppContext` (see `app.rs`).

## Tests

- `cargo test -j 8 -p gitvolt-gpui`, and `CEF_PATH=$HOME/.cache/tauri-cef cargo clippy -j 8 -p
  gitvolt-gpui -p gitvolt-core --all-targets -- -D warnings` before each commit.
- In a test module: `use std::prelude::v1::test;` (`gpui::*` brings its own `test`). GPUI's test
  scheduler panics when another thread wakes one of its tasks, so a test can't await `self.rt`: test
  the state and the views directly, and keep `GitVolt::listen` out of them.
- The core's own tests stay in `gitvolt-core`. Eight of them fail on this machine, on `main` too (lfs,
  rebase signing, watch busy-inactive): not yours.
- Idle costs nothing (`AGENTS.md`): no animation or timer that runs while nothing changes.
