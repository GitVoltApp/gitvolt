# GPUI rewrite: first slice and measurements

**Status:** in progress on the `gpui-rewrite` branch, beside the CEF app, which stays the product.
`crates/gitvolt-gpui` draws GitVolt's main view natively from the same `gitvolt-core`. It reads and
doesn't write. **The rewrite's goal is the current look, 1:1; most of the app isn't there yet.**

## What the slice has

Title bar with window buttons and resize edges, repository tabs, the toolbar's layout, the sidebar's
five panels, the graph table with bands, lines, nodes, chips and the collapse zone, a commit's basic
details, the status bar, reloads on the core's events, and the default dark theme. Sizes and colours
are the web UI's, read from its CSS and from the running app.

Not there yet, by size of the work: the diff and file views (Monaco, Shiki), every write (the
toolbar's buttons are disabled), the MR/PR views, Markdown, Mermaid and PDF, settings, profiles and
other themes, avatars, the command palette and menus, find, tooltips, the details panel's file list,
branch presets, text selection, accessibility roles, spell check, zoom, and macOS and Windows
checks. The core still runs on a temporary data directory until this build writes.

## Dependency

`gpui` and `gpui_platform` are pinned to Zed's `v1.27.0` commit, as
[zed-industries/zed#66275](https://github.com/zed-industries/zed/issues/66275) advises: crates.io's
0.2.2 is a year behind. Both are Apache-2.0. Zed's `editor`, `multi_buffer`, `language`, `markdown`
and `ui` are GPL, so GitVolt can't use them; the diff editor has to be its own. Every crate the
binary links on Linux is under an allowed license, checked from `cargo metadata` against `about.toml`.

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
- **The start times are warm ones.** The first GPUI launch of a series took 1465–1610 ms in three
  series, against 1312 ms for CEF's first run here, so a cold start may gain little or nothing. The
  cause isn't investigated yet.
- **Left out:** those first launches, two of which also showed 2.7–5.2% idle CPU with 69–126 frames
  (not investigated; it must be before this ships, since an idle app costs nothing), and one run
  with the window in the background, where GPUI held it at 24 Hz by design. The measuring build
  turns that throttle off.

To repeat the GPUI side, with `GITVOLT_T0` the launcher's clock in ms:

```bash
GITVOLT_T0=$(date +%s%3N) GITVOLT_BENCH=1 GITVOLT_OPEN=<repo> target/release/gitvolt-gpui
```
