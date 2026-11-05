# GPUI rewrite: first slice and measurements

**Status:** in progress on the `gpui-rewrite` branch, beside the CEF app, which stays the product.
`crates/gitvolt-gpui` draws GitVolt's main view natively from the same `gitvolt-core`. It reads and
doesn't write. **The rewrite's goal is the current look, 1:1; most of the app isn't there yet.**

## What the slice has

Title bar with window buttons and resize edges, repository tabs, the toolbar's layout, the sidebar's
five panels and its folded strip, the graph table with bands, lines, nodes, chips and the collapse
zone, the status bar, reloads on the core's events, and the default dark theme. Sizes and colours are
the web UI's, read from its CSS and from the running app.

The details panel has a commit's ids, every parent as a link, its author and message, and its files, by
path (with Sort by status) or as a tree, with "View all files" and its filter row, and the arrows and
Left, Right, Enter, Space, Home, End and the page keys once the list has the keyboard. Up and Down move
the area last clicked in: the graph, the file list, the history's list or the diff.

The diff has Inline, Split and Hunk, File View, word-level marks, the overview ruler, indent guides,
"ignore whitespace", the next and previous change (F7), selecting and copying text, word wrap, folds
and File View's minimap. It colours with a TextMate tokenizer on Oniguruma, `vscode-textmate`'s algorithm
with injections, and Shiki's grammars and theme. Against Shiki on real files of this repository (Rust,
TypeScript, TSX, CSS, Markdown, TOML, JSON, YAML, Python, about 392,000 characters), not one character's
colour differs; the test's samples cover 34 languages, 21 of them for their injections.

File History and Blame open from the toolbar, Ctrl+Shift+H and Ctrl+Shift+B, in the graph's place: the
commits, paged, File | Changes over the file at a commit or its diff, the resizable list, and the blame
gutter whose groups pick their commit (Alt: in the graph). A binary shows in the hex view (16 or 8 bytes
a row, changed bytes by position, the bar's marks); an image in Side-by-side, Swipe, Onion skin or
Difference with zoom, pan, Match sizes, the backgrounds and its Hex or Source toggle. "Open in" lists
every opener, as the app's `Api` wires them.

Not there yet, by size of the work: every write (the toolbar's buttons are disabled, and so hunk and
line staging, and editing in a diff), the MR/PR views and review comments, Markdown, Mermaid, PDF and
media views (a PDF or media file shows as hex), settings, profiles and other themes, avatars, the
command palette and menus, find, tooltips, branch presets, accessibility roles, spell check, zoom, and
macOS and Windows checks. In the diff: moved-code detection and code lens. The core still runs on a
temporary data directory until this build writes.

Differences from the web UI that are known, in the area that's there:

- **Text:** a line's advance is 7.8 px, the font's; Chromium rounds it to 8. Lines end a little earlier
  and wrap a little later. The hex view paints on Chromium's 8 px grid. UI text is a pixel or two wider.
- **Diff:** the first change is centred about 10 px lower than Monaco's; the ruler sits beside a 10 px
  column where Monaco overlays its scrollbar on the text; the toolbar stays one row when it's too narrow
  instead of wrapping. Monaco's taller hidden-lines row is its 24 px here too.
- **Embedded languages** load as a grammar names them. Shiki leaves a lazy one plain until it's been
  loaded, which depends on what was opened before: a Markdown fence is coloured here from the start.
- **File History:** the list's width isn't remembered; a file with no history closes the panel instead
  of opening over it; going back and forward to it isn't there.
- **Blame** has no tooltip. Blame from Diff View opens on Changes, with the toggle off, as the web UI does.
- **Images:** AVIF isn't decoded; a GIF shows its first frame; an SVG draws through `usvg`, not Chromium;
  a side over 4096 px holds a smaller copy, and above 100% stays smooth where the crisp copy would pass
  that; zooming out samples linearly. No right-click menu, no keyboard on the swipe handle, and the
  background pick lasts the session. As in the web UI, only Ctrl+wheel zooms.
- **Hex:** the panes don't select or copy, and have no find.
- **Open in:** an editor starts from this process's environment, since this build doesn't capture the
  login shell's; the opener from Settings isn't read; the last one used lasts the session; stored
  versions' copies go in a `gitvolt-gpui` cache folder of their own.
- **File list:** View all files is for commits, not the WIP row (`Request::WorktreeFiles`). Folder rows
  have no change counts or guide lines. A text field's caret stays at the end, with no IME.
- **Commit:** no committer row, edit button or copying an id; a parent that isn't loaded says so instead of
  loading more of the graph.
- **Toast:** one kind, for a few seconds, no details.

## Dependency

`gpui` and `gpui_platform` are pinned to Zed's `v1.27.0` commit, as
[zed-industries/zed#66275](https://github.com/zed-industries/zed/issues/66275) advises: crates.io's
0.2.2 is a year behind. Both are Apache-2.0. Zed's `editor`, `multi_buffer`, `language`, `markdown`
and `ui` are GPL, so GitVolt can't use them; the diff editor has to be its own. `image` (MIT/Apache) was
already in the lockfile through gpui. Every crate the
binary links on Linux is under an allowed license, checked from `cargo metadata` against `about.toml`.

`assets/generate.mjs` makes what's embedded from the web UI's npm packages, so both draw the same:
252 grammars (1.9 MB gzipped: the 233 `ui/src/diff/shikiLanguages.ts` keeps for their licenses, and the
internal ones Shiki loads with them for injections), the theme, the icons and the editor font's Latin face (OFL-1.1). What that face lacks falls back to the
system's, unlike the web UI's other subsets. The release notices don't cover this build yet.

## Measurements

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

- **The GPUI build does far less.** It has no diff view, no MR views and few of the features whose
  code the web UI loads at startup. Its memory will grow as they arrive, so ~9× is an upper bound
  on the final gain, not a forecast. The same goes for size: it has no forge layer yet.
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
  (not investigated; it must be before this ships, since an idle app costs nothing), and one run
  with the window in the background, where GPUI held it at 24 Hz by design. The measuring build
  turns that throttle off.

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
- **Since then:** with File History, the hex and image views and Open in added, two runs on the same
  kind of file show 7–24 / 12–33 ms to open, a 6.95 ms median and 9.37–9.46 ms worst frame, 2.27–2.36 ms
  of CPU per frame, 0.05–0.10% idle CPU with no frames, and 60–73 MB anonymous memory, up from 56.

To repeat the GPUI side, with `GITVOLT_T0` the launcher's clock in ms:

```bash
GITVOLT_T0=$(date +%s%3N) GITVOLT_BENCH=1 GITVOLT_OPEN=<repo> target/release/gitvolt-gpui
```

`GITVOLT_SHOW=<graph row>,<file>[,hunk|split|file|wrap|history|blame|swipe|onion|difference|fit|zoom4|menu|allfiles]...`
opens a file without an input device, for screenshots and for the diff's run above.
