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
  under `target/bench/work/`, so the user's settings are never read or written. Tabs restore from the profile there: a script's first state is what the last run left, so delete `xdg/config/gitvolt-gpui` for a clean start.

## How it's put together

- **State:** `GitVolt` (`app.rs`) holds the tabs, and `render` draws everything from it. Sub-views are
  functions taking `&mut Context<GitVolt>`. A diff lives in `Tab::diff` (`diff/view.rs`).
- **The core:** call `Api` from `self.rt.spawn(..)`, await the handle in `cx.spawn`, then
  `this.update`. `api.dispatch(Request::X { .. })` gives JSON (camelCase, as `ui/src/api/gen/` types
  it). Writes go through the same `Request`s, so the core's write pipeline and journal apply. Add a
  typed method to `Api` when a payload is big, as `Api::graph` does.
- **Writes:** `src/write.rs`, the port of `runWrite`. `self.write(Write::new(&ctx, move |asked| call("method",
  json!({ .. }))).origin(origin).on_success(..).on_end(..), cx)`: `call` builds a request as `ui/src/api/client.ts`
  does, a question from the core arms at `origin` and sends again with `asked`, an error toasts, the answer feeds the
  journal, staging and WIP lists. Test it in a `#[tokio::test]` with `write::testing::Fixture`, a throwaway repo through
  a real `Api`; a GPUI test can't await the runtime, so under `cfg!(test)` writes land in `GitVolt::sent`.
  `wip.rs`, `commit_box.rs`, `sync.rs`, `refs.rs`, `feedback.rs`, `manage.rs`, `gitflow.rs`, `submodules.rs`, `stacks.rs`, `lfs.rs`, `merge/`, `irebase/` and `diff/staging.rs`, `diff/edit.rs` hold them.
- **A modal's `build` and an overlay run while `GitVolt` is being updated:** reading it there panics. Copy what it
  draws into something shared before `ui::overlays`, as `sync_activity_shown` does.
- **What floats:** `src/ui/` is the shared UI; `ui::init` once, `ui::begin_frame` first in the root's
  `render`, `ui::overlays` as the root's last children. Its state is the `Ui` global, so anything with
  an `App` can open or close it. What's in it:
  - `ui::input::TextInput`: `cx.new(|cx| TextInput::new(cx).placeholder("Filter"))`, a child like any
    other; `TextInput::multiline` for a textarea. It tells its owner through `InputEvent` (`cx.subscribe`).
    Up, Down and Esc go on to the parent unless the owner claims them (`claim_arrows`, `claim_escape`).
    A subscription has no window: to move the keyboard from one, set `GitVolt::focus_next_frame`.
  - `ui::tooltip::tip("text", child)`: at once, under the target. `.delay(ms)`, `.placement(..)`, and
    `Anchor::new(child).card(key, build)` for rich content (a hover card). A target needs no id.
  - `ui::anchor("name", child)` records where a control is, for what's placed against it.
  - `ui::menu`: rows are `MenuRow::action/submenu/header/Separator`, `.shortcut`, `.variants`, `.swatch`,
    `.disabled(why)`. `MenuRow::pending` holds a row's place until its action is ported. Open with
    `menu::open_context(rows, e.position, window, cx)` from a press, or `menu::open_below("anchor", rows,
    ..)`: from `on_mouse_down`, not `on_click`, so press, drag and release picks. `src/menus.rs` has
    what each right-click shows: wire a write by replacing its `pending` row.
  - `ui::arm::request(req, Origin::control("anchor"), then, window, cx)` arms a control in place; in a
    menu row's `run`, pass `Origin::current(cx)`, with `arm::hold_row` and `release_row` around a read that comes
    first, so the row can still arm. With no control (`Origin::None`) it asks in a popover. `.icon(name)`: the label keeps an icon button's icon.
  - `ui::picker::open_below(key, items, placeholder, toggle, on_pick, ..)`: `RefPicker`, a searchable list under a button; `open` at any box. It owns the keyboard while it's open.
  - `Modal::anchored(bounds)`: a panel dropped from a control, with no shade behind it.
  - `ui::popover::ask_choice`, `ui::modal::open` and `ui::modal::prompt` (a dialog takes the keyboard
    in its own subtree, so no shortcut acts behind it), `ui::toast::show` and `show_with`,
    `ui::banner::banners`, and `ui::controls`: `button`, `select`, `switch`, `check_row`, `keycaps`,
    `clearable`, and `tabbable`, which gives a dialog's control a Tab stop.
  - `ui::fuzzy::go` ranks as `fuzzysort` does.
- **Commands:** `src/commands.rs` is the one table of actions: id, label, group, icon, tooltip, chords,
  when and run. The hamburger, the chords, the toolbar's tooltips and the palette read it; a row with no
  `run` shows disabled until it's ported. To add a chord, add a row: bare keys stay GPUI bindings.
- **The palette** can open as a picker: `GitVolt::choose(Choices { .. })`.
- **The profile and the settings** are `GitVolt::profile` and `settings`, JSON as the core says them (`Request::LoadState`); `update_profile` and `update_repo` change and save. A graph read carries its preset's `view` and `pin`: `graph_options`.
- **Sizes a drag sets** go through `GitVolt::save_layout` (`src/layout.rs`): those the web UI has in the profile go there, what it has in localStorage stays in `layout.json`, `prefs` among it.
- **Drawing:** `theme.rs` has the shown theme's colours, every `tokens.css` token by the CSS's name: `theme::blue()`, never a stray hex, and never a colour kept in a struct across frames (a theme switch only refreshes). `theme::editor()` has what Monaco draws with. Built-in themes come from `assets/themes.json` and `assets/editor-themes/`: `assets/generate.mjs` writes them, and the goldens in `src/testdata/` and `src/diff/testdata/`. Paint-heavy views use `canvas`, as `graph.rs` and `diff/view.rs`.
- **Sizes that follow a setting** are functions, `metrics::row_h()`, `line_h()`, `advance()`, `chip_h()`, `file_row_h()`, `panel_bar_h()`; `settings/effects.rs` applies a change.
- **Tabs and repositories:** `Tab::id` and every variable named `repo` in the views are a tab's id, not the core's. What goes to the core goes through `app::core_repo(id)` (`Request`s and `"repo": ..` in a `call`); an event from it goes to `tabs_on(repo)`. `Tab::state_id` is the profile's id for the tab; `save_tabs` writes the strip to the profile.
- **Settings:** a setting is a row in `settings/pages.rs` (`Def`: label, page, group, keywords, control, binding). What it does goes in `settings/effects.rs`, not in the row. `set_settings` and `update_profile` save through the core. `settings_field` gives a page a text box by id.
- **Colour picker:** `ui::colour::swatch(key, value, label, alpha, size, on_preview, on_commit, on_cancel, cx)`; `theme::preview` shows a value without saving it.
- **Zoom** is the platform wrapper in `zoom.rs`, generated once from `gpui`'s `Platform` and `PlatformWindow` traits: when the pinned `gpui` moves, new trait methods need a line of delegation there.
- **Icons:** add a Lucide name to `ICONS` in `assets/generate.mjs`, run it from `ui/` (`node
  ../crates/gitvolt-gpui/assets/generate.mjs`), then `icon("name", size, color)`.
- **Text:** the UI font is `system-ui`'s; mono text is `diff::view::FONT`. There's no `letter-spacing`
  in GPUI; uppercase captions go without it. The root's line height is relative (1.4), as the CSS's: a smaller text has a smaller line.
- **No input device here:** `GITVOLT_SHOW` (`bench.rs`) opens things by script for screenshots
  (`GitVolt::show_ui`: `menu3`, `sub7`, `hamburger`, `palette=q`, `find=q`, `filter=q`, `prompt`, `armed`;
  `-` for no selection or no file); extend it as needed. Clicks and keys are checked in tests with GPUI's `TestAppContext` (see `app.rs`).
  `GITVOLT_SCRIPT=<file>` runs a flow a line at a time, each input through the window as the platform's would be:
  `click x y`, `rclick`, `dclick`, `move`, `press`, `drag`, `release`, `key ctrl-s`, `type text`, `wait ms`, `select row`,
  `open index`, `cmd id`, `show <ui part>`, `mdscroll y`, `pdfscroll y`, `wheel x y`, `wheelup x y`, `ctrlwheel x y`, `goto <text>` (what a paste of it would go to; the clipboard stays alone), `pickfolder <path>` (the next directory picker answers this; under a script no real dialog ever opens), `shot out.png`, `dump` (a JSON line of what's on screen), `quit`; and by name, `pick <row text>` (a menu's or a picker's),
  `pickv <row>|<variant>`, `hover <row>`, `clickon`, `shiftclickon`, `ctrlclickon`, `rclickon`, `dclickon`, `presson`, `dragto`, `releaseon <end of an
  anchor's key>`, `clickrow`, `shiftclickrow`, `ctrlclickrow`, `rclickrow`, `dclickrow <summary>`. `tools/run.sh <repo> <script>` runs one isolated: config, data and cache
  under `target/bench/work/xdg`, no avatars, no global git config. Only ever on a throwaway repo under `target/bench/work/`.
  `tools/md-fixture.sh` makes `md-fixture`, with old and new versions of Markdown, PDFs, a WAV, videos and fonts, and diagrams. `tools/conflict-fixture.sh merge|rebase|irebase` makes one that's stopped in a merge, stopped in a rebase, or ready for a plan. `dump`'s
  `merge` and `rebase` say what those tools hold, `settings` and `profile` what those hold, `zoom` the zoom. `GITVOLT_FAKE_UPDATE=<state>` shows the update pill in that state, asking no host. Their anchors: `merge-hunk:<side>:<n>`, `merge-line:<side>`, `merge-take-all-<side>`, `merge-save`,
  `merge-next`, `irow:<oid>`, `iaction:<oid>`, `ichip:<oid>:<branch>`, `ichip-add:<oid>`, `rebase-start`, `rebase-cancel`, `op-abort-at`, `op-continue-at`.

- **Real input:** `tools/realinput.sh <repo|-> <command...>` starts a nested, invisible KWin with its own Xwayland and runs the build in it
  unscripted, so it starts as a user's does (`NATIVE=1`: on Wayland, the desktop's default; `LAYOUT=de`: that keyboard). The app there has no D-Bus, its own HOME and XDG folders,
  a plain `sh` login shell and a `PATH` of an allowlist with `xdg-open` and the like faked (what they're asked goes in `$OPENED`), its working directory away from this repository, and `GITVOLT_TEST_FORGE=off` unless a flow sets it: a click can't reach the desktop or a real host.
  `tools/realinput.py` sends a device's events through the compositor (`move`, `click`, `dclick`, `down`, `up`, `wheel`, `scroll`, `key Ctrl+p`, `raw 100+16` by evdev code, `hold`, `release`, `type`, `wait`), one device for a run; it refuses to run without the nested socket. `tools/wshot.py out.png [x y w h]` takes the picture, on Xwayland and on Wayland, by KWin's own call on the nested compositor's private bus.
  `GITVOLT_DEBUG_PIPE=<fifo>` (off without it; `realinput.sh` sets it) makes the app answer a line on the fifo with what's on screen in `<fifo>.out`: `dump`, every anchor, a menu's and a picker's rows with where to press, the window, the cursor last asked for, the last input events, where a frame's time went.
- **Flows:** `tools/flows/run.sh [name...]`, from the repository's root, runs the suite, each in a nested compositor of its own, on Wayland too where a flow says `# native: yes`; it stops on nothing and prints each flow's count. A flow is a shell script over `flows/lib.sh`: `up <repo>`, `clickon <anchor>`, `clickrow <summary>`, `pick <row>`, `pickv <row> <variant>`, `key`, `ri <realinput verbs>`, `q <python over d>`, `check`, `check_sh`, `until_q`, `shot name`, `finish` (fails on any panic in the log). `flows/session.sh` holds a compositor open, to find out a flow a command at a time. `flows/monkey.sh` is random input (`MONKEY_SEED`, `MONKEY_SECS`); give each run its own `REALINPUT_ROOT`. Under real input the fake device's wheel has no notch (`value120`): through Xwayland it's a notch by the numbers, on Wayland GPUI drops it, so a wheel on Wayland can't run here.
  `GITVOLT_SCRIPT` skips the single instance and more: a flow that only ever ran scripted hasn't shown that the app starts.
- **What's cached:** `main.rs`'s `Root` holds `GitVolt` as a cached view, as Zed caches a pane, with `ui::spin`'s layer over it. Everything that's a child of `GitVolt` still redraws it when it notifies. What turns goes through `ui::spin::spinner`, never GPUI's `with_animation`, which is a frame of the whole window at the display's rate; `spin::cover()`, the first child of what floats, hides what turns under it.

## Tests

- **Every run of a test binary or of the app goes under a memory cap and a timeout**, so a runaway fails on its own and can't take the machine down (one has, holding 20 GB):
  `( ulimit -v 10000000; timeout 300 cargo test -j 8 -p gitvolt-gpui --target-dir target/bench )`. If the cap kills cargo or rustc while building, build first with `cargo test --no-run` without it, then run under it. The same around a scripted run of the app (`tools/run.sh`), raised only if a GPU driver's address space needs more. A repository's file is untrusted: a loop that walks it or grows a buffer has a bound, pinned by a test.
- **What takes a repository's bytes and can't be bounded from the inside runs in `src/worker.rs`** (a PDF, a diagram), under its own memory limit and timeout. What runs in the app's process (Markdown's parsers, a font's tables) has a bound on every loop and buffer, and a test on it.
- `cargo test -j 8 -p gitvolt-gpui`, and `CEF_PATH=$HOME/.cache/tauri-cef cargo clippy -j 8 -p
  gitvolt-gpui -p gitvolt-core --all-targets -- -D warnings` before each commit.
- In a test module: `use std::prelude::v1::test;` (`gpui::*` brings its own `test`). GPUI's test
  scheduler panics when another thread wakes one of its tasks, so a test can't await `self.rt`: test
  the state and the views directly, and keep `GitVolt::listen` out of them.
- The core's own tests stay in `gitvolt-core`. Eight of them fail on this machine, on `main` too (lfs,
  rebase signing, watch busy-inactive): not yours.
- Idle costs nothing (`AGENTS.md`): no animation or timer that runs while nothing changes.

## The forge

Only ever the harness's fake forge, on 127.0.0.1, with its fictional seed and fake tokens: no fixture or run reaches a real host, or the real keyring.

- `tools/forge-lab.sh start|stop|base|fixture|profile|account|requests|seed|reset`: the fake forge, fixtures whose origin is named by a forge URL but reads a local bare repository, and clean profiles. `stop` ends the exact pid it started.
- `GITVOLT_TEST_FORGE=<base>` points this build's forge hosts at it, with tokens in a file in the data folder and links printed (`openUrl <url>`) rather than opened. `off`, or any scripted run, also keeps off the keyring. `tools/run.sh` sets it to `off` unless it's given.
- `tools/forgeref.mjs <repo> <steps.mjs> <out-prefix>`: the reference: the web UI's e2e bundle in Playwright's Chromium, on the same harness, with the same host rules as the e2e.
- `dump` has `forge`: kind, list, badges, the open MR/PR, its threads, a review, the Create flyout. Their anchors: `side:mr:<n>`, `mr-*`, `rv-*`, `create-*`, `stack-*`, `fork-add:<path>`, `toast-action:<label>`. A scripted window may never be the active one, so a menu doesn't close for that under a script.
- `GITVOLT_XDG` must be absolute. A rewritten stack only pushes with a lease when its commits are the fixture's identity's: give the fixture the author's `user.email`.
