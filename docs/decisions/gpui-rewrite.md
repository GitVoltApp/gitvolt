# GPUI rewrite: first slice and measurements

**Status:** in progress on the `gpui-rewrite` branch, beside the CEF app, which stays the product for now.
`crates/gitvolt-gpui` draws GitVolt's main view natively from the same `gitvolt-core`. It reads, and writes
through the same `Request`s as the web UI, so hooks, config, credentials and the journal behave the same.
**The rewrite's goal is the current look, 1:1. The app is ported but for what "Not there yet" and "Differences from the web UI" name, the largest being sounds and films (no playback), and it has run on Linux only.**
The branch is on GitVolt 1.6.0 and main at 50ddc69 (sections below).

## GitVolt 1.6.0

What 1.6.0 adds beyond main at 55263b6, `git diff 55263b6..v1.6.0`:

- **Ported, main's 50ddc69:** an action key in the rebase editor moves on to the row after the last selected one, unless it opened a message.
- **Ported:** a file's Discard runs once until its answer (`discardOnce`), its row with a spinner in its Discard's place and its Stage and menu rows waiting; Discard unstaged one at a time.
- **In the core, so here too:** git started without a fork, the quicker one-file discard, the first account's keyring name.

## The diff's tints, against Monaco

Compared on one fixture (a word changed, lines removed and added whole, a block of unequal sides) in Inline, Split and Hunk, sampled pixel by pixel, the CEF 1.5.0 package beside this build. A line removed or added whole tints its row, but inline a removed line's tint ends with its text; a paired line with no words of its own changed stays plain; the old numbers' strip keeps the removed tint beside an added line. What differs still: a block of unequal sides pairs its lines by shared words (`align_lines`), not Monaco's character diff, so which words of a paired line are marked can differ, and Split puts a block's hatch after its lines where Monaco may put it between them. Nothing marks the caret in the strip beside the ruler, where Monaco has a tick.

## GitVolt 1.5.0 and main (55263b6)

What the web UI gained since the port's base (261a31a), hunk by hunk of `git diff 261a31a..55263b6`:

- **Ported, Title bar (`src/chrome.rs`, `settings/pages.rs`):** Settings › Appearance › Title bar: Separate, Combined, System, above Fonts, its bullet description, Restart GitVolt under it once the choice isn't what the window was built with, in a line that keeps its space, ↺, and an X11 session's fixed System with its note. Combined: no title bar row, the space before ☰ (gone maximized), 28 px buttons, tabs at a smaller minimum, 48 px before the bell the strip never covers, the window buttons in the tab bar, a double click on the strip that's a title bar's. The restart is the core's `relaunchApp`, for real: `main.rs` wires an updater with no source of releases (`NoReleases`) only for that, so nothing's ever asked of a host; a scripted run has none, and says to start it by hand.
- **Ported, button groups:** Density and Title bar as joined buttons (`frame::choice_group`), one Tab stop, the arrows wrapping, Home, End, Space, Enter, the ring by the keyboard only.
- **Ported, Restart asks:** `confirm_discard_unsaved`, the web's "Discard unsaved changes?" with Discard edits arming, before the update dialog's Restart and the Title bar's, the dialog staying if it's cancelled; settings, tabs, drafts and sizes saved first. Closing the window now asks the same one question, as the web's does, where it used to walk the dirty tabs with a Save.
- **Ported, buttons that move the window:** Open repository, the bell, Settings and the window's own (those hadn't had it): a press that moves past 4 px moves the window, a click presses. The bell opens on the click, and a second click closes it, as there. The title bar and the strip's empty space wait for the same 4 px now.
- **Ported, main:** several accounts on a host, by id: the default marked, the same user's token updating theirs with its notice, Replace token naming the user and refusing another's, Remove saying what else changes, the form's line that keeps its place, `accountChoice.ts`'s functions with its tests, Settings › Repository › Forge account where it's a choice, a repository's account as who the views act as (`me`, Assign to me). The rest is the core's. Keyring: the first account keeps its entry's name, under this build's own service.
- **Checked, main's two fixes:** Split's first change: Monaco's two editors drifted; here both sides are one list of rows with one scroll, so it can't, and a test pins it on the e2e's file shape. The find widget's buttons: a tooltip here takes no pointer, so it can't cover them; a flow rests on Next and Close until the tooltip shows and then presses.
- **Doesn't apply:** the macOS traffic lights (positions, `traffic_lights`, the display notification) and Windows' relaunch (the core's), `win-titlebar.ps1`, the demo's files. GNOME's button layout and title bar clicks were never read here: the window buttons stay on the right, a middle click on a title bar does nothing, a double click always maximizes.
- **Not ported, the window coming back on screen when a monitor goes:** GPUI's window has `resize` but no way to move it, on any platform here, and on Wayland a client can't anyway. It's the compositor's, as the position has been all along.
- **Found while comparing, and fixed:** a graph of few lanes was 44 px wide where the web's `autoGraphWidth` keeps 64, and a chosen width wasn't clamped to the lanes; Alt+click on a sidebar folder folded it instead of hiding what's under it, and a hidden folder's row wasn't marked; an armed control's caption squeezed to a letter a line; Esc in Settings lost the keyboard after a removal; with the system's title bar the body still left room for the row it hasn't.

## What the slice has

Title bar with window buttons and resize edges, repository tabs you can drag along the strip, the
toolbar, the sidebar's panels (filter, sort, collapse, dividers, width, the narrow strip, hover cards),
the graph table (columns that resize, hide and are remembered, chips with `+N`, the WIP row, the collapse
zone and its shade, Load older commits, the dim on resting on a branch's chip), avatars, the status bar,
reloads on the core's events, and every theme. Sizes and colours are the web UI's, read from its
CSS and from the running app.

The shared UI every later feature builds on is in `crates/gitvolt-gpui/src/ui` (its README says how to
use it): text fields and textareas with IME, tooltips and hover cards, context and dropdown menus, arming in
place, popovers, dialogs, select, switch, checkbox and keycaps, toasts and banners. One table of commands
(`src/commands.rs`) drives the shortcuts, the hamburger and the command palette, which ranks as `fuzzysort`
does; `/`, `@` and `>` work. Ctrl+V goes to the commit, file or branch the clipboard names, and Ctrl+F finds in
the graph or an open file. Right-click menus are there on graph rows, chips, the header, the sidebar, tabs, the
tab bar, file rows and a diff's text, with the web UI's rows and tooltips; Copy, Compare, Open in, File history,
Show in graph and the like work, and every row that needs a write or a dialog holds its place, disabled.

The details panel has a commit's ids, every parent as a link, its author and message, and its files, by
path (with Sort by status) or as a tree, with "View all files" and its filter row, and the arrows and
Left, Right, Enter, Space, Home, End and the page keys once the list has the keyboard. Up and Down move
the area last clicked in: the graph, the file list, the history's list or the diff.

The diff has Inline, Split and Hunk, File View, word-level marks, the overview ruler, indent guides,
"ignore whitespace", the next and previous change (F7), selecting and copying text, word wrap, folds
and File View's minimap. It colours with a TextMate tokenizer on Oniguruma, `vscode-textmate`'s algorithm
with injections, and Shiki's grammars and themes. Against Shiki on real files of this repository (Rust,
TypeScript, TSX, CSS, Markdown, TOML, JSON, YAML, Python, about 392,000 characters), not one character's
colour differs; the test's samples cover 34 languages, 21 of them for their injections.

File History and Blame open from the toolbar, Ctrl+Shift+H and Ctrl+Shift+B, in the graph's place: the
commits, paged, File | Changes over the file at a commit or its diff, the resizable list, and the blame
gutter whose groups pick their commit (Alt: in the graph). A binary shows in the hex view (16 or 8 bytes
a row, changed bytes by position, the bar's marks); an image in Side-by-side, Swipe, Onion skin or
Difference with zoom, pan, Match sizes, the backgrounds and its Hex or Source toggle. "Open in" lists
every opener, as the app's `Api` wires them.

## What writes

`build_api` wires the `Api` as the app does: the login shell's environment, askpass, and a data folder of its
own (`gitvolt-gpui`, beside the CEF app's, never shared). Requests are built as `ui/src/api/client.ts` builds
them and go through `src/write.rs`, the port of `runWrite`: a question the core answers with arms where the
action started and sends again, an error toasts, the answer updates the journal, the staging state and the lists.

- **The WIP panel:** its header and discard, View all files, Path and Tree, the filter, Conflicted, Unstaged and
  Staged with their resizer, stage, unstage and discard of a file, a folder or everything, staging's own Undo and
  Redo, per-worktree drafts (the WIP row's input and the commit box are one text, kept in `drafts.json`), the commit
  box with Amend, the identity line and Ctrl+Enter, the operation box (status, Continue, Skip, Abort, each stop's
  message once, a merge's message added to the draft and taken back on Abort), and Mark resolved.
- **The diff:** hunk rows with Stage, Unstage and Discard, the line button in the gutter, staging a selection,
  the working copy edited in place (Inline and File View), Save, Stale and the unsaved-edits question on leaving a
  file or closing the window.
- **The toolbar:** Undo and Redo with their history menu, Fetch or the chosen Pull, Push (force with lease, the
  upstream panel, a rejection's choice), Branch, Stash, Pop, the WIP button, split buttons' hold and
  press-drag-release.
- **Feedback:** the core's op events, the action queue with Resume and Clear, spinners only while working, running
  marks on chips and sidebar rows, the activity log, Debug's command list, the bell, the status bar's items, background
  fetch on the settings' interval, refs events reloading, banners with their actions.
- **Credentials:** the askpass modal (one at a time, a password's characters as stars), the switch-to-SSH offer, a
  background fetch's "authentication required" in the status bar.
- **Branches, tags, commits:** checkout (with a chip's double click, the sidebar's, diverged and autostash questions),
  create, rename, delete with its remote, merge, rebase, fast-forward, cherry-pick, revert, reset, set upstream, tag
  create, delete and push, a branch dragged onto a chip, and the real Fast-forward, Merge and Rebase rows.
- **Managing:** add and remove a remote, Copy URL, create, remove and switch a worktree, Create file, a submodule's update.
- **The merge tool (`src/merge/`):** a conflicted WIP file opens it in the diff's place: Current and Incoming over the output, each with its hunk
  column, line numbers, the hovered line's take or drop button, minimap and ruler; ticks by hunk, by line (Space, Shift+Space for the hunk) and by side;
  the output edited by hand, its regions tracked as Monaco tracks them, a tick an undoable edit that takes the ticks back too; F7 and the arrows;
  Ctrl+S or Save and mark resolved, arming for what's left unresolved; a stale save's Reload from disk or Overwrite; a file deleted on disk; binary,
  delete/modify and other non-text conflicts' buttons; a conflicted row's Take current, Take incoming, Mark resolved and a folder's; moving to the next
  conflicted file when one's resolved, however it was; the unsaved-work question on leaving, closing a tab or the window.
- **Interactive rebase (`src/irebase/`):** the editor in the graph's place with the sidebar away and the selected row in the details panel: pick, reword,
  squash, fixup, drop, edit by the dropdown or the keys, Ctrl+arrows and dragging a row or a selection, the message edited in place, predicted conflicts,
  chips dragged to a row, added, struck out and reverted, Undo and Redo of the plan, Reset, Reload after RefMoved, Start. From a branch's menu, a dropped
  chip's menu, "after this commit", Drop commit and the palette. An Edit stop in the box: commit in pieces, Continue under it, what Abort keeps. Each stop
  selects the WIP row once, and an operation that ended outside GitVolt settles its paused entry.

- **Selecting graph rows (`src/selection.rs`):** a press selects, Ctrl toggles, Shift and Shift+arrows take a range from the anchor. Two rows are a compare (with the
  swap, the two sides' rows and View all files on its TO), a commit and a WIP row compare it with that tree, three or more are the multi-selection summary. A right-click
  inside one has the selection's menu: Squash N commits, Squash N commits interactively…, Cherry-pick N commits, Revert N commits, Interactive rebase from here. Picks
  follow their commits across a reload; a reload keeps what's on screen where it was. Home, End, the page keys, → and Enter on the first file, the menu key.
- **The graph's rows:** a body's first line after the summary, the SHA that copies itself, the branch's dimmed chip on a hovered or selected row and the branch focus by its
  membership (`branchMembership`), the avatar at Author's minimum, another worktree's WIP row taking a double click; a press beside the chips doesn't select.
- **The commit's panel:** the split between it and its files (dragged, double-click resets, in `layout.json`), ids that copy, the committer's row, the author's own date.
- **The Open Repository screen (`src/open.rs`):** an Open tab, with no tab open or from Ctrl+O, the folder button, a double click on the tab strip: the default-directory banner,
  Recent (pinned, removed, filtered, in the profile and the palette), Open directory…, Clone (the destination following the URL, progress, Cancel, its errors), Your repos
  (directories added and removed, refreshed, A–Z or newest, one list or grouped). What's opened or cloned takes the Open tab's place; one that fails says so on the screen.
- **Inline creation and pickers:** Create branch here, the toolbar's Branch and Create tag here are `src/inline.rs`'s field over the row's chips, with its hint or error floating
  below, Ctrl+Enter for create only, an annotated tag's message under it; the dialog only where there's no row. `ui::picker` is `RefPicker`: the toolbar's branch list with its order
  toggle, Set upstream, the header's pin. The push panel drops from the button. Add remote takes the URL first and the name follows its owner. Create file's inline field sits on top of
  its list, a folder prefilled, and the new file opens in File View. Create worktree has Browse… and Open in a new tab; Remove worktree has With branch; "checked out elsewhere" its links.
- **Git LFS (`src/lfs.rs`):** a file row's mark, Track and Stop tracking, LFS: Pull and Fetch, the status bar's "isn't installed", a diff's "not downloaded" with Download and Show pointer.
- **Git flow (`src/gitflow.rs`):** the Git Flow panel, Set up and its settings, Start with its kinds, Finish with its tag and message, a conflicting merge's toast and its Merge action.
- **Submodules (`src/submodules.rs`):** Add, Update, Sync URLs, Open in a tab (the palette's list, the panel's double click, a file row), Initialise, Remove, the panel's menu and +.
- **Stacks (`src/stacks.rs`):** detection as `detect.ts`, Push stack bottom first with its one toast, Rebase stack onto its base with its armed row and what it says stays behind.
- **Branch presets and Hide (`src/presets.rs`):** the button beside the sidebar's filter with its cycle and menu; what a graph read carries (`view`, and the `pin`); Hide and Show of a branch,
  tag or folder from the menus, the sidebar's eye and Alt+click, on a built-in in the repository's Custom; the ⋮ rows, fold and hidden marks, the reveal panel with Expand in graph, Collapse branch, Fold merged branch,
  the hidden list with Show and Show all; the pin's picker.
- **File colours (`src/filecolours.rs`):** the patterns compiled by hand as `glob.ts` does, the rows' tints by `rowTint`, the menus' Scope line and Colour rows.
- **The Fork trunk notice** above Commit, with Create branch… and Dismiss.

- **Themes (`src/theme.rs`):** the ten built-ins, from `assets/themes.json`, which `assets/generate.mjs` writes from `themes.ts`; `derive` and `customThemeDef` ported, golden-tested on a custom theme per base; each editor theme's tokens and Monaco's colours in `assets/editor-themes/`, against Shiki on ten samples per theme. Every `theme::x()` reads the UI thread's palette, cheaper than the `rgb` conversion it replaced; a switch swaps it and refreshes, and an open file or merge tool is read and coloured again, keeping unsaved typing. The saved theme applies before the first frame. Lanes and their text, file tints and Chromium's range colours follow the theme.
- **Settings (`src/settings/`):** the overlay, its nav and search with marks, deep links with the flash, Reset all and ↺, all the pages but Accounts' content. The theme page has the cards, the strip (rename, Copy as JSON, Export file…, Delete arming), Paste a theme and Ctrl+V, the lanes dragged or changed, every colour with its picker (`ui/colour.rs`, OKLCH, previewing on the app), All colours and the floating Edit mode. The font picker lists the installed families, previews beside it, and takes a custom list. Branch presets (`presets/settings_page.rs`) has the list with its ticks and drags, rules with live counts, picks, options, sharing and Edit presets… in the button's menu. File colours (`filecolours/settings_page.rs`) has scopes, chips, tints, Start collapsed (which a tree now honours), counts, try a path, share and copy from another repository.
- **What the rows do (`settings/effects.rs`, `metrics.rs`):** density (row, chip, lane, file row and panel bar sizes), UI font, editor font, size, line height, ligatures, tab width, whitespace, minimap; commit limit; date format and relative dates, ticking once a minute only while on; avatars asked again; fetch interval applied live, off, the first fetch at once when it's due, a tick missed while hidden replayed on coming back; Open in's default from the repository's or the profile's editor, a custom command checked by the core; hosts, trunk, pin.
- **Profiles (`src/tabs.rs`):** the switcher, New, Edit, Delete arming in its submenu; switching asks about unsaved work, closes the tabs and brings up the other profile's. Sidebar and details widths, sidebar panels and each repository's columns live in the profile; the chosen Pull in the settings. As in the web UI, drafts, merge drafts, the WIP split and the details split stay beside the app (`drafts.json`, `layout.json`), per machine, not per profile.
- **Tabs and window:** a tab's id is the core's repository id with the tab's number above it (`app::core_repo`), so several tabs can show one repository, each its worktree: Open in a new tab and Go to its tab from Create worktree, the checked-out-elsewhere toast, the sidebar's and WIP's menus. Tabs, the active one, closed tabs and names persist and come back at the start. Tab groups: chips, folding, the chip's hover list, colours, names, New tab in group, Save and close, Ungroup, Delete arming, saved groups in the tab bar's menu. A dragged tab's preview and slide, into and out of groups, are `src/tabstrip.rs`, the port of `useTabDrag`. The window's size and maximized state are kept. Zoom scales through the platform (`src/zoom.rs`): Ctrl+=, Ctrl+-, Ctrl+0 and the status bar's menu, sharp as a browser's; over an open file they change its text size. A later launch hands its path to the running one (`gitvolt_core::instance`). Settings › Appearance › Title bar is read at the start.
- **Keyboard Shortcuts (`src/shortcuts.rs`):** the panel and Settings › Keyboard, from the command table and the web UI's hints, the rebase editor's bare keys included. The web UI has no rebinding, so neither has this.
- **Debug:** Requests from the core's log, Actions as commands run, Copy diagnostics. **Git check:** the blocking screen when git is missing or too old. **Update pill and dialog (`src/updates.rs`):** every state, on the core's `UpdateState`.

## Markdown, PDFs, sounds, films and fonts

Markdown (`src/md/`) is pulldown-cmark into a tree of its own, the web's plugins (GFM autolinks, GitLab image sizes, emoji, references, slugs, front matter), then its own HTML tree builder and sanitizer with the tables of `sanitize.ts`, boxes, and a layout that follows Chromium's: margins collapsing, half-leading, whole-pixel advances, UAX #14 breaks, table auto-layout. Off the UI thread up to the boxes; lazily by top-level block after that; a selection, links, images with the same consent and rules as the web, `details`, Tab and Enter on what's focusable. File View and Diff View have the Source and Rendered toggle, inline and split with its ruler, and Previous/Next change and F7. The field (`field.rs`) is Write and Preview with its toolbar and completions; the lightbox (`lightbox.rs`) is the web's keys and wheel. A repository's Markdown is untrusted: every loop over its text is bounded, and the tests pin each one (`ast.rs`, `frontmatter.rs`, `safe.rs`, `layout.rs`).

**What reads a PDF or draws a diagram runs in a process of its own** (`src/worker.rs`), this same binary with `--render-worker`, as pdf.js has a Web Worker: `hayro` and `mermaid-rs-renderer` take a repository's bytes and can't be bounded from outside except this way. The worker has a 3 GB address space and no core dump, and a request past its time (20 s for a page, 10 s for a diagram) kills it; the view then says so. A flate-of-flate PDF unpacking to 8 GB took the worker to 2.1 GB and failed it, and the app kept running. A document's worker goes with it, and exits when its pipe closes.

- **PDF (`src/pdf.rs`):** pages, Fit width and the slider's ladder, Ctrl+wheel and a double click for 100%, the keys Ctrl+= Ctrl+- Ctrl+0 (the Keyboard Shortcuts panel lists them), Page N, Hex, Old and New side by side in step page by page, only the pages near the view drawn, each page within pdf.js's 16 megapixels. On generated files (embedded fonts, a gradient, a clip, dashes, a JPEG, half alpha, Multiply, rotated, landscape and small pages, an owner-password file, 300 pages) `hayro` matched pdf.js to the eye in every image compared.
- **Mermaid (`src/md/mermaid.rs`):** `mermaid-rs-renderer` drew all eight kinds tried (flowchart, sequence, class, state, ER, gantt, pie, gitGraph) correctly, in the web's dark or default colours. It's drawn from a picture of its SVG, so nothing in it can run or load; a label can't add markup (a test pins it). A `.mmd` file shows its source, as the web's.
- **Sounds and films (`src/media.rs`):** GPUI has no player and a decoder's a heavy dependency, so there's no playback and no frame. A pane says so, with what the header holds (a WAV's or FLAC's length, rate and channels; an MP4's or QuickTime's length, picture size and codec), Open in and Show hex.
- **Fonts (`src/media.rs`):** the sample and Basic Latin's grid are drawn from the file's own outlines (`ttf-parser`) as one picture, never added to the app's fonts, so a repository's font can't change the app's text, and they can't collide with its families. TTF, OTF and WOFF (each table unpacked within a bound).

## Not there yet

By size of the work: spell check, and macOS and Windows checks. In the diff: moved-code detection and code lens. In the forge: comments on a rendered Markdown diff's blocks, and a forge-hosted video in a body (`forgeVideo`, `forgeOpenVideo`).
In the details panel: "vs 1st parent" on a merge.

The forge's flows ran by script against the harness's fake forge only (GitLab, GitHub, Forgejo), never a real one; by real input against the same fake forge, GitLab only.

## Real input

Everything above first ran from the tests and from `GITVOLT_SCRIPT`, which sends events through the window. Since then the app has run unscripted in a nested KWin, with a device's events through the compositor (`tools/flows/`, README), and that found bugs no script could: the app panicking on every start, the wheel at 81 px a notch in the graph and 117 in a diff, a mouse drag selecting nothing in any editable file, a tab not following its worktree, Enter in the profile dialog panicking. What this covers is what the flows check, in the fixture's git state, the fake forge's log or the app's own state, and pictures that were read.

- **Run on Xwayland and native Wayland (`NATIVE=1`), by the suite:** first start, start with a path, tabs restored, a later launch handing its path to the first and exiting 0, quit by the window's button with no socket left, closing with unsaved edits (Save, Esc, Discard), window size kept, maximize and restore, minimize and a later launch bringing it back; a drag's, a double and triple click's and Shift+click's selection in a diff, Ctrl+C into the commit box, a drag past the text's edge; every shortcut of the command table by chord, the arrows, Home, End and the page keys in the graph, Shift+Down; the German layout's AltGr characters and umlauts in a field and `Ctrl+,` `Ctrl+-` `Ctrl++` `Ctrl+=` `Ctrl+/` `Ctrl+0` `Ctrl+Shift+1..3` by physical keys; a spinner's cost and hung fetch's Cancel; a stale save's Overwrite and Reload, a hidden tab's unsaved close, undo and redo in an editor, Discard hunk, Stage hunk; the panel, tab, chip and rebase row drags and Esc in them, the split button's press-hold and press-drag-release, the merge tool's ticks, panes' selections and scroll sync by the ruler and minimap, Rendered Markdown's drag selection (`drags`); fast-forward, a chip drag's menu row, soft and mixed reset, Set upstream, a tag's push and delete on a remote, Copy URL, Remove remote confirmed, the compare's swap; cherry-pick and revert stops through the merge tool; Create worktree, a worktree row's double click, checked-out-elsewhere's links, Remove worktree With branch; saved tab groups, profile delete; system title bar, the git-too-old screen, a folder dialog with no portal; Alt+click; details toggle and a live theme switch with a diagram open; submodule Update, Initialise, Sync URLs, Remove; a file and a repository deleted while open, quick clicks, the smallest window. Random input: about 2,200 actions on each backend and 2,500 more on Xwayland, 1,000 on each after the parity round, and about 1,350 on each on 55263b6's final code, no panic and no hang.
- **Run on Xwayland only:** the wheel, by the suite in the graph, a diff, the sidebar, a file list and a menu, by hand in Markdown and the merge tool's scroll sync; 4000 commits under the wheel; the right edge's and a corner's resize, a window drag from the title bar and a double click on it; the rebase editor's Undo and Redo of the plan, bare keys, Ctrl+Up and message by double click; the palette's Interactive rebase with an upstream; Rendered Markdown's F7 and ruler; submenus by hover and arming's cancel, by hand. Against the fake forge: an account by the settings page, the MR view's wheel, a note, reply and resolve, a note's edit and delete, a reviewer added, the reaction picker's search, Approve's arming, Create MR's flyout opening.
- **Added by the 1.5.0 round, on both backends unless it says (`titlebar`):** Separate, Combined and System across real restarts by the button, the new process checked to be this binary in the isolated folders; Restart with unsaved edits, Esc and Discard edits; the groups' keys and Tab; what moves the window (a position only on Xwayland); the bell's two clicks; 30 tabs in Combined; maximized; an X11 session's fixed System, Xwayland only. On Xwayland only (`accounts`, `leftovers`): two accounts on the fake forge, a repository picking the second, a restart keeping it, Remove's caption, no token in the log, profile or dump; the banner's Use, Queue Clear and Resume, Alt+click on a folder, the update dialog's Restart with unsaved edits (under `GITVOLT_FAKE_UPDATE`, so the core then says there's no update to restart into), auto-merge set and cancelled, Discard draft, forks' second page.
- **Added after the first use by hand (`ruler`, on both backends; `reviewrange`, `mergebtn`, Xwayland):** the overview ruler, File View's minimap and the graph's scrollbar held without the view moving, pressed off the ruler centring there, a drag going on off the strip until it's let go; the open file's row closing it and opening it again; a maximized window's edges doing nothing; a drag from the "+" over three lines, blue while it goes and yellow with the box open; GitHub's Merge split button's height, divider and hover, beside the CEF's in a picture.
- **Added by the parity round (`strip`, `rows2`, `clone`):** a tab's drag, into, out of and onto a group, a group by its chip, Esc in a drag, the chip's list, Rename in place; settings' Tab stops and the trap, a page's scrollbar track, a lane and a rule dragged, a theme switch with a source diff open, find's toggles, the Colour row's strip, sticky scroll; a clone's Cancel under a hung `git`; the Create flyout's target and draft.
- **Not run by real input:** a wheel on native Wayland (the fake device has no notch and GPUI drops it; `zoom.rs`'s conversion has a unit test); IME (no input method in the nested compositor); resize edges on Wayland; a clone's credentials; LFS Download (no LFS storage in a fixture); git flow's conflict toast; Rebase stack with merges, a stack push failure; a successful Choose on the Open screen's banner (the nested compositor has no portal to answer); swipe handle, PDF ruler and font bars; `<video>` in Markdown; forge: file, tag, Monaco and remote forge rows, Use as the main remote, the push toast's link, the remote picker, stacks on GitHub and Forgejo, settings and Hosts rows, GitHub's and Forgejo's views; a selection's primary selection across another program; the CEF reference wasn't run under the same input, so "as the web's" here means its code, unless the section below says it was compared.

## Compared by eye with CEF

**1.5.0, the release package unpacked, its update check and avatars off, nothing connected from it,** in a nested compositor at 1600×900, the same clicks on both: Separate and Combined, one tab and 18, a tab hovered with its tooltip, Settings › Appearance with the offered restart, both groups and the keyboard's ring, the Open screen with a URL typed, a hidden folder's row and its graph. Its font here isn't this build's (Chromium's `system-ui` with no desktop session), so text widths differ by a few pixels; everything else lands on the same pixels but the 1-2 px below. Maximized and System were only looked at in this build's pictures. By reading only: the popover Restart opens (the flow's picture is under the button, as `askChoice` places it), a real Combined drag on CEF, and all of main's accounts UI: no package exists for 55263b6, so Accounts and Forge account were matched to `AccountsSection.tsx`, `repository.tsx` and their tests, not by eye.

The same window size (1600×900), the 1.4.1 build (261a31a) in a nested compositor, and a picture of each read:

- **The main view:** graph and details, a source diff inline and split, File View with the find widget, Rendered Markdown's diff; a diagram.
- **Themes:** Light, Monokai, Darcula, GitHub Dark and Solarized Light, over a source diff; the theme page in Light.
- **Settings:** every page (General, Appearance, Theme, Graph, Branch presets, Diffs and editors, Fetch and push, Keyboard, Advanced, Git identity, Editor and folders, Accounts, Hosts, Repository, File colours, with a scope in it).
- **Floating things:** the profile menu, the branch picker, the presets menu, Add remote, Create worktree, Add submodule, Set up git flow, the palette, the hamburger, the Open screen.
- **In Playwright's Chromium on the fake forge** (`forgeref.mjs`; the CEF build can't be pointed at one, and headless hides scrollbars): the MR/PR view on GitLab, GitHub and Forgejo.

Not side by side: the fork notice, the Stack panel and Create stack MRs (the reference's fixture wouldn't offer the row, and the release can't be pointed at a fake forge), the ⋮ hidden list (a hidden folder's row was, the list wasn't reached on the fixture), clone progress once it's running (the fixture's hung `git` didn't start in the release: Clone stayed disabled there with no destination, and this build's picture is the only one).

## Differences from the web UI

Known, in the area that's there:

- **Title bar:** Linux's wording only: the Windows and macOS descriptions aren't kept, as nothing else of those is run. An X11 session is `WAYLAND_DISPLAY` unset and `XDG_SESSION_TYPE` not `wayland`: this build on Xwayland inside a Wayland session counts as Wayland, as CEF on Xwayland does. A choice's hover has no fade, the chosen fill does. A folder row's hover eye isn't there (Alt+click and the menu are).
- **Accounts:** a restored strip keeps two tabs of one repository, where the web's drops the second (only a hand-edited profile has them).
- **Writes, dialogs:** Tab reaches a prompt's checkbox and buttons, but no select. An annotated tag's message box has no resize. A clone's progress bar is drawn, not Chromium's. Create worktree's Browse… sits beside its field; the web's is an unstyled grey button a whole row wide, not copied.
- **Writes, questions:** a question from a write's own answer, when the menu row that began it has closed, is a
  popover (asking first, as a preview does, holds the row and arms it, as there). A linked worktree's tab moves to the main
  one rather than a twin closing. Undo's menu opens once its entries are read. Finish's and Start's radio buttons and Set up's fields are the app's, drawn alike.
- **Selecting rows:** the compare's sides and the multi list take their tooltips on the summary only, as there, without the web's keyboard path into them. The menu key opens the row's menu at its left under the row.
- **Open screen:** the window's title says the Open tab rather than the last repository's. Below 900 px wide the three columns don't stack. A group of "grouped" lists scrolls with its column rather than by even shares of the height.
  Tab reaches the fields, Recent's rows, Open directory and Clone, not the pin and remove buttons or Your repos' rows. The directory picker doesn't start where the web's does: the portal is asked for no start.
- **Presets:** The ghost stub's hover zone on the graph isn't there: the "⋮ N hidden" text opens the list on a press, or Enter on the row; so the list needs a press, not a hover. The reveal panel's rows have no right-click, and no
  Up and Down inside it; a ⋮ row's dotted lane opens it after the same rest as the row, not at once; a fold's panel opens on a press, not a hover. It doesn't curve the merge's row into its lane. H on a sidebar row isn't bound (the eye and Alt+click are). Going to a commit a preset hides (`revealCommit`) says it's not loaded rather than unfolding it.
- **File colours:** A dragged scope moves at once, without the slide. The menu's Scope line is text, with no swatch.
- **LFS:** the "not downloaded" notice is the diff's; a File History row's sides use the same read, so they show it too.
- **WIP and commit box:** a conflicted row's label sits under the row's hover buttons, as there.
- **Merge tool:** the editors are the plain one under Editing below: no find (Ctrl+F does nothing there), no multiple cursors, and their right-click has Copy,
  and Paste in the output, not Monaco's whole menu. The work lives in the open tool: every way of leaving asks first, so there's no store of drafts behind it,
  it goes with the app (the web UI's goes with its window's session), and without work coming back to a file opens at its first conflict rather than where you
  were. An undo of a hand edit takes its region's "edited" mark back too. A hunk's checkbox and Take all aren't Tab stops: Shift+Space on a pane's line takes its
  hunk, and Take all from Current or Incoming are in the palette. The minimap draws its characters as bars; the ruler's thumb shows while
  the pointer's over the editor, not on a scroll. The unsaved and stale questions are the centred popover. No fade on a line button; its + and − are icons.
- **Interactive rebase:** The action dropdown's rows have no tooltip beside them while the menu's
  on the keyboard. Start, Reset and Take all's keyboard way is the palette, not Tab. The rebasing chip doesn't float its full name on hover. A deleted chip's and a dropped row's line is drawn over the text, a little under where Chromium's goes. A stop's
  question from Drop commit (the autostash) is a popover, as it comes after the row's menu closed. The title's chips don't truncate with an ellipsis.
- **Editing:** a plain editor, not Monaco: no multiple cursors, no suggestions, no blink; Up and Down go by logical line,
  and so do Home and End in wrapped text. Colours catch up 120 ms after typing. A press on a removed line doesn't
  move the caret. An inline selection can take removed lines too.
- **Activity log:** Requests and Actions are plain lists, without the web UI's filters and copy-all; there's no performance overlay. This build's log files are in its own cache folder (`gitvolt-gpui/logs`), with Debug logging and Logs folder on them. Actions lists commands, not every menu row a view adds by itself.
- **Rendered Markdown:** raw HTML goes through a small parser rather than Chromium's: no adoption agency, only text is fostered out of a table, about 110 named entities, a nesting limit of 100 (Chromium's is 512, and past it they both flatten). `rowspan` is ignored, `ruby` is inline, a picture's `align` doesn't float. Footnotes number across the whole file, where the web's chunks each start again, and no margin breaks at a chunk's end. A checkbox and the disclosure triangle are drawn alike, the focus ring is near; emoji and ↩ come from the system's fonts, a little different in their advances. Positions are within a pixel of Chromium's after a scroll (its scroll snapping), the pane's content 0.19 px off. Find doesn't look in rendered text. `<video>` in Markdown is a box that opens the page or Open in; the `@user` hover card and the flavour by forge come with the forge views. A selection across blocks is by the text's order.
- **Markdown field:** no resize handle on the textarea; compared against the web's styles and markup injected into a CEF page, as a forge view needs a forge.
- **The forge (`src/forge`):** every view here was compared with the web UI in Playwright's Chromium (the engine the e2e runs on, at the same viewport) on the e2e bundle against the fake forge, by `tools/forgeref.mjs`, not with the CEF build, which can't be pointed at one. Headless hides its scrollbars. Compared by image: Accounts (add, connected), the sidebar's section and badges, the MR/PR view on GitLab, Submit review and the cards, Add remote's forks, Create on GitHub. Not compared: Create stack MRs, the Stack panel, the reaction picker's search, GitHub's and Forgejo's MR/PR views (looked at, no reference beside them). None against a real forge.
  - Rendered bodies are `crate::md`'s, with its differences above. The text gutter of a commented diff is 98 px wide against Monaco's 116; a card is a block over a blank row of the scroller, not a Monaco zone. Comments on a rendered Markdown diff's blocks aren't here: its source view takes them.
  - A suggestion's Apply is a row under the note, not in the suggestion's caption, and lines that moved since say so on the click, not by greying it first. In a pending comment's preview a suggestion shows only what it adds.
  - Tab in a flyout and on review cards walks the controls in view by a ring, and Enter or Space presses the one it's on, instead of moving real focus; PageUp and PageDown scroll, so what's out of view is a page away. A flyout's scrollbar is a thumb that shows where you are; the wheel moves it, a drag doesn't.
  - A people cap learned from a refusal (`learnLimit`) isn't kept for the session: the project's limits are read as on the web, and the refusal is the toast. Create's Template asking to replace edited text says so in a toast, and choosing it again replaces it. Create stack MRs is a modal, as the web's. Push stack's server warnings inside the after-merge flow show in its own toast only for that moment.
  - Keyring: this build keeps forge tokens under the service `gitvolt-gpui` in the system keyring, and never touches the app's `gitvolt` entries. Only `GITVOLT_TEST_FORGE` or a scripted run takes the file store, in its own data folder.
- **Lightbox:** no video; above 100% it's smooth rather than pixelated; a wheel notch steps once, whatever its size.
- **Mermaid:** side by side with CEF the web's diagrams fill the column too, so the size is as there; a small diagram is scaled more here, as the crate lays it out narrower (a box 308 px wide with a larger label against the web's 373). The layout and shapes are that crate's, not mermaid.js's: rounded corners, no glow, thicker arrows, different spacing, DejaVu or the system's font rather than `trebuchet ms`. It's more forgiving: a sequence line with no target draws where mermaid.js says it's an error. The picture is drawn once at the column's width, up to 4×, so a pane widened later keeps its width; each diagram is drawn in turn from the top rather than as it comes near the view, the first after about 700 ms (the worker's fonts). One diagram's timeout gives up on the rest of that file, which stay as source. The error line doesn't wrap, and isn't selectable.
- **PDF:** no text layer, so no selecting, copying or finding in a page. No form widgets or annotations but what `hayro` draws, and encrypted files with a user password say so, as the web's. After switching from Diff to File View the web's Fit width stays at 100% until a resize (a measure that comes too soon there); here it fits at once. A page that fails stays blank paper, as there, but a worker that's died fails its whole side, with the error.
- **Sounds and films:** nothing plays, no first frame, no controls, also for what Chromium would play (WAV, FLAC, MP3, Ogg, WebM, H.264 where it has the codec): the note says "This build can't play…". An MP4 or QuickTime's H.264 or HEVC note is the web's words. The header's details are an addition, which the web's player doesn't show. A WebM says no codec.
- **Fonts:** WOFF2 isn't read: its tables are one Brotli stream with transforms of their own, so the pane says the font couldn't be loaded. Glyphs are outlines without hinting, a little lighter than Chromium's at small sizes. The samples have no colour-font or variable axes support, beyond the default instance, and no shaping: a pangram in Latin needs none.
- **Toolbar buttons of a PDF, film or font** aren't Tab stops, like the image diff's. Their keyboard way is Ctrl+= Ctrl+- Ctrl+0 for the zoom, and Show hex in the palette. A font's pane scrolls across by the wheel or its bar only.
- **Credentials:** a password field shows stars rather than the browser's dots.
- **Text:** advances are whole pixels, as Chromium's on Linux at a device scale of 1 (`src/glyphs.rs`, round each glyph's own advance FreeType's way, add its kerning, round again, as Blink's `RoundHarfBuzzBufferPositions`): 319 of 320 measured widths (8 strings, 10 sizes, 4 weights) equal CEF's, where 41 of 320 did before, and the one left is 3 px on a line of kerning pairs at 12.5 px. A long line at 13 px is 494 px in both (it was 507). Glyphs are still GPUI's rasterizing, not Skia's: a stem can fall on another pixel.
- **Diff:** on opening a file the first change is about 14 px lower than Monaco's (F7 lands on the same pixel); the removed side's
  number column isn't tinted beside an added line; the toolbar stays one row when it's too narrow
  instead of wrapping. Monaco's taller hidden-lines row is its 24 px here too.
- **Embedded languages** load as a grammar names them. Shiki leaves a lazy one plain until it's been
  loaded, which depends on what was opened before: a Markdown fence is coloured here from the start.
- **File History:** the list's width isn't remembered; a file with no history closes the panel instead
  of opening over it; going back and forward to it isn't there.
- **Blame** from Diff View opens on Changes, with the toggle off, as the web UI does.
- **Images:** AVIF isn't decoded; a GIF shows its first frame; an SVG draws through `usvg`, not Chromium;
  a side over 4096 px holds a smaller copy, and above 100% stays smooth where the crisp copy would pass
  that; zooming out samples linearly. No right-click menu, no keyboard on the swipe handle, and the
  background pick lasts the session. As in the web UI, only Ctrl+wheel zooms.
- **Hex:** the panes don't select or copy, and have no find.
- **Open in:** the last one used lasts the session; stored versions' copies go in a `gitvolt-gpui` cache folder of their own.
- **File list:** View all files is for commits, not the WIP row (`Request::WorktreeFiles`). Folder rows
  have no change counts or guide lines.
- **Text fields:** the caret blinks ten times after it moves or the field takes the keyboard, then stays lit, since an idle app costs nothing; Chromium's blinks for as long as the field has focus. There's no spell check (`spellCheck` is Chromium's; a native one would need a dictionary dependency, not added) and no right-click menu in a field.
- **Icons:** drawn from GPUI's 2× mask, so about 10% heavier in mean coverage than Skia's at small sizes (the bell's 57.8 against 52.3). Text isn't: its ink is within 1% (94.0 against 92.7), LCD on both.
- **Wrapping:** GPUI breaks before any non-word character (`Ctrl\n+-`), where a browser follows UAX #14; a wrapped description hangs its trailing space as a browser does (`ui::hanging`), elsewhere a last word within a space of the edge goes to the next line.
- **The wheel:** a notch moves at once, without Chromium's and Monaco's smooth scrolling. Through X11 (not the desktop's Wayland), GPUI's client learns a device's position from its first event after the pointer enters, so that first notch doesn't move anything.
- **Spinners** turn at 30 frames a second, not the display's, and are drawn over the app rather than in it, so a shade over one hides it instead of dimming it. A running pipeline's icon turns again: 3.1% of a core under X11 and 2.1% on Wayland, on a 60 Hz output, where a frame of the whole window at the display's rate took 18% (67% was seen at 144 Hz); the layer's rate doesn't follow the display's.
- **A deleted repository** leaves its tab showing what it last read: the next action's error says so (`failed to run git`), there's no banner, as the web's code has none either (read, not run side by side). A file deleted while it's open stays open until the next click.
- **Accessibility:** GPUI at this rev has AccessKit (`.role(..)`, `.aria_label(..)`, AT-SPI on Linux). Roles are on buttons (the shared control, the toolbar, the window's), tabs and their list, graph rows, sidebar rows, dialogs, menus, settings' switches and selects. Not on fields, a menu's rows, file rows, the forge's and the merge tool's controls. Never run under a screen reader: the nested compositor has no AT-SPI bus, so this is only that it builds and no flow breaks.
- **Primary selection:** a selection in a diff, a merge editor or rendered Markdown becomes the primary one, and a field, an editable diff and the merge output take it with the middle button.
- **A folder dialog** needs the desktop portal; without one it says so in a toast, where Chromium falls back to GTK's.
- **Tooltips and cards** fade in over 150 ms, as the web's. A card takes no pointer: nothing floats that the
  pointer can enter.
- **Menus:** a row's highlight doesn't fade in (the web UI's 50 ms). The Open in dropdown in the diff's toolbar is still its own menu, without
  press, drag and release; the same list in a file's right-click menu is the shared one. Tab groups' rows are absent or disabled. A menu's note row is told its line count.
- **Opacity:** `opacity` in GPUI is per quad and glyph, not the group's as in Chromium: a dimmed control's border and text sit over its own dimmed ground, a little brighter than the web's. A disabled Clone is mixed by hand to match; plain disabled buttons aren't.
- **Chips:** a truncated chip doesn't float its full copy on hover, a stack from `+N` is read-only (its right-click is on the `+N`, by submenus).
- **Graph:** a graph with too many lanes for its default width shows the collapse zone sooner than it would. No lane scrollbar under a clipped Graph column; a Branch/Tag column at its minimum shows chips as they fit rather than their icons alone. Column resizers and panel dividers are for the mouse, not the arrow keys. The SHA shows six characters however wide its column.
- **Settings:** the live preview on the theme page is a small drawing of the same tokens, not the graph's own rows and canvas. A press outside the colour picker commits and doesn't also reach what's under it. The Reading rows store and show their values, and take effect with the Markdown views; the forge rows (Merge requests target, Update an MR's branch by, From your forge accounts) likewise store only. Search folds Latin accents by a table, not Unicode's NFD. Setting focus on a deep link goes to the dialog, not the row's control. Tab reaches the fields, switches and selects of a page; a Custom control's own buttons (the theme strip, presets, file colours) aren't stops.
- **Themes:** an active indent guide isn't drawn in its own colour (the diff never drew one).
- **Tabs:** a group folds at once, without the web's width animation; the chip's line is drawn per tab, so in a slide it follows each one rather than easing as one; a chip isn't a Tab stop, so its list and menu have no keys; a wheel scrolls the strip at once, without its glide, and a drag near the strip's edge doesn't scroll it. Position isn't restored (Wayland doesn't let a client), only size and maximized.
- **Tabs' restore:** a tab whose repository can't be opened any more stays in the strip and the profile, on the Open screen saying why, where the web's stays with an error in the graph's place.
- **Palette:** an action that isn't ported is listed and says so when picked.
- **Go to:** a forge page or number goes by the tabs already shown: a tab not shown yet isn't opened to look in it, as the web's is. A `file://` line takes no drive letter.
- **Find:** a full hash outside the loaded window says so rather than loading until it appears. In a file
  it's Monaco's widget, drawn alike, with its case, word, regex and in-selection toggles; a pattern is the `regex` crate's, not JavaScript's; no replace row.
- **Commit:** the signature is its icon with a tooltip, not the badge's card; a parent that isn't loaded says so instead of
  loading more of the graph.

## Dependency

`gpui` and `gpui_platform` are pinned to Zed's `v1.27.0` commit, as
[zed-industries/zed#66275](https://github.com/zed-industries/zed/issues/66275) advises: crates.io's
0.2.2 is a year behind. Both are Apache-2.0. Zed's `editor`, `multi_buffer`, `language`, `markdown`
and `ui` are GPL, so GitVolt can't use them; the diff editor has to be its own. `image` (MIT/Apache) was
already in the lockfile through gpui. Every crate the
binary links on Linux is under an allowed license, checked by `cargo about generate --offline --locked --fail -c about.toml -m crates/gitvolt-gpui/Cargo.toml` (cargo-about 0.9.2, under `target/bench/work/tools`), the first time that's been run on this crate: nothing refused, on 593 crates: 537 MIT, 19 Unicode-3.0, 16 Apache-2.0, 10 BSD-3-Clause, 6 ISC, 3 Zlib, 2 CC0-1.0, 2 MPL-2.0 (`option-ext`, `uluru`: the allow-list's weak, file-level copyleft), 1 BSD-2-Clause, 1 CDLA-Permissive-2.0. No copyleft is chosen: the only GPL in an expression is `self_cell`'s `Apache-2.0 OR GPL-2.0-only`, used as Apache-2.0. `about.toml`'s `private = { ignore = true }` leaves out what isn't published, so Zed's crates (`gpui_*`, `collections`, `refineable`, `scheduler`, `sum_tree`, `http_client`, all Apache-2.0 by `cargo metadata`) aren't in that list and need adding by hand or by a config of this build's own. A release of this build isn't wired up; `docs/licensing.md` says what its notices would be made from.

`assets/generate.mjs` makes what's embedded from the web UI's npm packages, so both draw the same:
252 grammars (1.9 MB gzipped: the 233 `ui/src/diff/shikiLanguages.ts` keeps for their licenses, and the
internal ones Shiki loads with them for injections), the theme, the icons and the editor font's Latin face (OFL-1.1). What that face lacks falls back to the
system's, unlike the web UI's other subsets. The release notices don't cover this build yet.

The Markdown round added `pulldown-cmark` (MIT), `toml` (MIT/Apache), `yaml-rust2` (MIT/Apache), `hayro` (Apache/MIT), `mermaid-rs-renderer` (MIT) and, already in the lockfile through gpui, `ttf-parser` and `unicode-linebreak`; every new crate under them is MIT, Apache-2.0 or BSD-3-Clause (`cargo metadata` against `about.toml`). The release binary went from 101 MB to 115 MB with `hayro` and `mermaid-rs-renderer`.

## Measurements

The ported app against the 1.6.0 release, `tools/benchvs.sh`, by turns on one generated repository (4000 commits, 2000 loaded, a 3000-line file with two changes in the working copy), each in a nested KWin at 1600x900 on a virtual 60 Hz output, with the desktop in ordinary use (load 1-3). Three runs each after a warm-up that doesn't count. Timed from pictures, about 30 ms apart; memory and CPU from `/proc`, all the processes of a build. No frame times: a release of the CEF build has no port to ask.

| | CEF 1.6.0 (Xwayland) | GPUI (Xwayland) | GPUI (Wayland) |
|---|---|---|---|
| Start to graph rows, warm | 1003-1055 ms | 648-1138 ms | 373-665 ms |
| Open a 3000-line diff, first in the process | 549-593 ms | under the method's limit | under the method's limit |
| Idle CPU, graph | 0.40-0.45% | 0.20-0.25% | 0.05-0.10% |
| Idle CPU, diff open | 2.1-2.3% | 0.1-0.2% | 0.0-0.1% |
| CPU while the wheel turns, 20 notches a second | 71-73% of a core | 9.9-11.5% | no wheel there (below) |
| Memory after settling, PSS | 572-574 MB | 149-153 MB | 145-147 MB |
| The same, anonymous | 434-436 MB | 54 MB | 54 MB |
| The same, RSS summed | 1352-1357 MB, 10 processes | 236-240 MB, 1 process | 224-226 MB, 1 process |
| With the diff open, PSS | 688-697 MB | 159-162 MB | 152-154 MB |
| GPU memory | 60 MiB | 92 MiB | 92 MiB |
| Size | 369 MB installed | 122 MB binary, not stripped | same |

A diff's time runs from the click's input process returning, so it starts up to some 30 ms late: GPUI's readings were 4-7 ms, which says only that it's drawn within that; CEF's 549-593 ms is good to about the same. Start times spread more on this run than the first's (620-637 and 386-409 ms a day earlier), so the desktop's load shows in them. The wheel's row isn't one quantity: CEF animates each notch at the display's rate, GPUI moves at once, so GPUI draws about 20 frames a second there where CEF draws about 60. Per frame that's near what the table below found. The fake device has no notch on Wayland. CEF ran with its system title bar (an X11 session) and `system-ui` from no desktop session. Not measured on the real desktop at its 144 Hz, and not with a forge account; a first run after a boot will be slower for both.

The table below is the first slice's, kept for its frame times:

Same machine, repository and run for both: 24 threads, RTX 2080 SUPER, KDE on Wayland, about 144 Hz;
a generated repository of 4000 commits and 22 branches (`ui/e2e/perfRepo.ts`) with 2000 commits
loaded; a 1600×900 window at scale 1. Settle 5 s, idle 20 s, then 8 s of scrolling 16 px per frame.
Both are optimized builds without LTO. The CEF build has debug assertions on in `gitvolt-app` and
`gitvolt-core`, since that's what opens its DevTools port.

| | CEF (3 runs) | GPUI (4 runs) | Factor |
|---|---|---|---|
| Start to graph rows, warm | 1107–1312 ms | 375–409 ms | ~3× |
| Idle CPU, all processes | 0.3–0.4% | 0.05–0.10% | ~4× |
| Scroll CPU per frame, all processes | 10.84–10.98 ms | 4.10–4.47 ms | ~2.5× |
| Scroll frame time, median / p99 | 6.9–7.0 / 7.1 ms | 6.95 / 7.13–7.25 ms | same |
| Scroll frame time, worst | 7.1 ms | 9.26–13.88 ms | CEF better |
| Frames over 20 ms | 0% | 0% | same |
| Anonymous memory after settling | 415–416 MB | 44–45 MB | ~9× |
| RSS after settling | 1176–1183 MB, summed over 9 processes | 204–205 MB (PSS 180 MB) | see below |
| GPU memory | 78–84 MiB | 91 MiB | CEF better |
| Size | 369 MB installed (`libcef.so` 260 MB) | 64 MB binary | ~5× |

### Reading the numbers

- **The GPUI build does less.** The table above was measured when it had only the main view, before the
  diff and everything since. It still has no MR views or settings, whose code the web UI loads at startup. Its
  memory has grown and will grow as they arrive, so ~9× was an upper bound on the final gain, not a forecast; the table above has what it came to.
  The same goes for size: it links only the forge layer's Gravatar client so far.
- **The summed RSS overcounts CEF:** its processes share `libcef.so`, and a sandboxed renderer's
  PSS can't be read, so there's no exact PSS for it. Anonymous memory is the fairer figure.
- **Scrolling looks the same:** both hold the display's rate with no frame over 20 ms. The gain is
  CPU, so battery and heat, not smoothness. A weaker GPU may show more; this machine can't say.
- **CEF's DevTools connection** was open while it scrolled and adds some of its CPU. It runs under
  XWayland, GPUI natively.
- **Git operations** weren't measured: they run the same core and the git CLI in both.
- **The start times are warm ones.** A first launch took up to about 1.8 s, against 1312 ms for CEF's
  first run. The time goes to the platform coming up (49 ms warm, up to 1255 ms cold) and the window
  and its GPU context (about 250 ms warm, 500 ms cold), before any of GitVolt's own code draws.
- **Left out:** those first launches, two of which also showed 2.7–5.2% idle CPU with 69–126 frames
  (not explained: the first launch after a build, in the nested compositor, gave 0 frames and 0.15% in 20 s, the same as the second, so it doesn't reproduce away from the desktop; it must be looked at there before this ships, since an idle app costs nothing), and one run
  with the window in the background, where GPUI held it at 24 Hz by design. The measuring build
  turns that throttle off.

### Rendered Markdown, PDFs, fonts

Same machine and build, under a script. The 5000-line Markdown file: first blocks in 51 ms, colours by 107 ms (the web's about 770 ms, roughly: its script sleeps were taken off), then 6–18 ms for the next files. Scrolling: 1150 frames, median 6.95, p99 6.99, worst 7.70 ms, no frame over 20 ms, 2.63 ms of CPU a frame. Idle on a file, a PDF or a font: 0.25–0.35% with a script's timer waiting and no frame drawn; the worker 0.00% and 16–17 MB. A window on a 300-page PDF at 180%, at page 70: 270 MB resident, holding the three pages near the view. Eight diagrams at the column's width: 404 MB.

### The diff, against Monaco

The same run on a file of about 5000 lines, with a change every few dozen lines: open it, open a
second file of that size, open the first again, then scroll about 18,000 px in 8 s. Monaco's side is
3 runs while the machine was quiet, GPUI's 4; each app's other runs had another program using a dozen
cores, and aren't counted. GPUI's numbers are its first piece of colour, which covers the first
screen here; Monaco's are when the screen's lines have their colours.

| | Monaco on CEF | GPUI |
|---|---|---|
| First open: text / colours | 592–685 / 627–718 ms | 8–24 / 18–29 ms |
| A second file: text / colours | 129–204 / 152–237 ms | 7–11 / 22–24 ms |
| The first again: text / colours | 104–127 / 106–129 ms | 9–15 / 10–15 ms |
| Scroll frame time, median | 6.9–7.0 ms | 6.94–6.95 ms |
| Scroll frame time, p99 | 7.0–13.9 ms | 7.29–13.95 ms |
| Scroll frame time, worst | 7.1–83.4 ms | 9.37–14.66 ms |
| Scroll CPU per frame, all processes | 4.5–5.1 ms | 2.48–2.91 ms |
| Anonymous memory, file open | 532–540 MB | 55–56 MB |
| Anonymous memory, after scrolling | 701–706 MB | 56 MB |

- **Opening is the clearest gain,** and it can be felt: a large file's first open goes from over half
  a second to a frame or two. Monaco's first open also loads its own code; that's a real cost of it,
  but it's paid once a session.
- **Colours run from the file's top down,** in pieces. A first change deep in a long file waits for
  the lines above it: about 0.4 s at the end of a 5000-line Rust file, while the text shows at once.
- **Monaco does far more** (moves, folding, its editing model), and this view will grow. The memory
  column is again an upper bound.
- **Not all CPU is alike:** Monaco scrolled by real wheel events, about 50 a second that it smooths,
  and GPUI by a set offset each frame, over about the same distance. Treat the factor as rough.
- **Idle:** a run with a file open shows 0.05–0.45% CPU and now and then a few frames, not always
  none. That's not investigated yet.
- **Since this round (themes, settings, profiles, tabs):** on the fixture, natively, warm: 0.15% idle CPU with 1 or 2 frames in 20 s, under XWayland 0.35 to 0.40% with 0 or 1 frames; the first launch after a build 2.7 to 3.6% with 40 to 82 frames, the one this page already leaves uninvestigated.
- **Since then:** with File History, the hex and image views and Open in added, two runs on the same
  kind of file show 7–24 / 12–33 ms to open, a 6.95 ms median and 9.37–9.46 ms worst frame, 2.27–2.36 ms
  of CPU per frame, 0.05–0.10% idle CPU with no frames, and 60–73 MB anonymous memory, up from 56.

To repeat the GPUI side, with `GITVOLT_T0` the launcher's clock in ms:

```bash
GITVOLT_T0=$(date +%s%3N) GITVOLT_BENCH=1 GITVOLT_OPEN=<repo> target/release/gitvolt-gpui
```

`GITVOLT_SHOW=<graph row>,<file>[,hunk|split|file|wrap|history|blame|swipe|onion|difference|fit|zoom4|menu|allfiles]...`
opens a file without an input device, for screenshots and for the diff's run above.
