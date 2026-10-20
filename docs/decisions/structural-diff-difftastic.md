# No structural diff (difftastic)

**Decision:** GitVolt doesn't ship a structural diff mode. One was built on
[difftastic](https://difftastic.wilfred.me.uk/) in October 2026 and dropped before release: it
added about 57 MB to every install and a separate binary to build, check and ship on every
platform, and on real changes it was rarely better than Monaco's own diff, and sometimes worse.

This records that work so a later attempt can start from it. Its code isn't part of this
repository.

## The idea

A toggle beside Ignore whitespace, "like hide whitespace, but superpowered": compare code by its
syntax, so lines only re-indented or re-spaced show as unchanged (a block wrapped in a new `if`
shows only the `if` and its brace as added) and only the tokens that changed are highlighted.
Staging had to keep working on git's real lines.

## What was built

- **difft, compiled in:** the `difftastic` crate (0.71.0, MIT) has no library API, only the `difft`
  binary, so it was built from the crates.io source (pinned, checksum-checked, `--locked`) by a
  script and shipped beside `gitvolt` in every package (deb, rpm, Arch, NSIS, MSI, the macOS app),
  cached in the release workflow.
- **Licences:** one grammar, `tree-sitter-newick`, is CeCILL-C (copyleft), so a build-time patch
  removed it. Every other crate in difft's tree (about 140) was MIT, Apache-2.0, BSD, Zlib or
  Unicode; four grammars vendored as C were MIT or CC0.
- **Size:** the binary is almost all tree-sitter parse tables, generated C per language. Stripped
  on Linux it was 117.6 MB; a second patch dropping eleven rarer languages (Verilog, VHDL, F#,
  OCaml, Fortran, Julia, Perl, Haskell, Apex, QML, Smali) brought it to 57 MB, +7.2 MB to the
  compressed `.deb`. Verilog alone was 17 MB of tables, F# 14 MB; TypeScript 2.8 MB, JavaScript,
  Python and Go under 0.5 MB each. A cold build took about 3 minutes.
- **The core read:** difft ran on the diff's two sides in a private temp folder (`DFT_UNSTABLE=yes
  --display json`), with a 4 s timeout, a fallback to the text diff for binaries, large files,
  unknown languages and parse errors, and the reason in the toggle's tooltip.
- **Drawing it:** Monaco 0.57 takes no diff from outside, so the editor worker service's
  `computeDiff` was wrapped for the model pair on screen.
  1. First, Monaco's changes were built from difftastic's `aligned_lines`. Two problems: in Split
     view Monaco realigns rows around each inner change, which put new lines above old ones until
     each token's other side was pointed outside the change; and difftastic's alignment is a
     display layout, not a line matching, so a line it called unchanged could be paired with a
     replaced line and show as changed and out of line where Monaco's diff kept it level. It also
     dropped Monaco's moved-code detection.
  2. Then the reverse: Monaco's own text diff (alignment and moves as in normal mode), refined. Inside
     each change, lines equal apart from whitespace with no difftastic token on either side were
     paired (an LCS) and shown as unchanged; highlights were difftastic's tokens.
- **Staging:** git stages lines, so "Stage chunk" took the whole git change groups a chunk touched
  (staging only some lines of a group reordered them in the index), and a row counted, staged or
  discarded the lines only reformatted.

## Why it was dropped

- **Token matching reads worse on prose and small edits.** difftastic matches tokens, so a doc
  comment with a few words swapped came out as every word changed, where Monaco's word-level diff
  marks just the swapped words. Comments, strings and small edits are most of what people read in
  a diff.
- **Little left once Monaco's diff is the base.** With the second design, what difftastic added
  over Monaco's diff plus Ignore whitespace was hiding re-indentation inside changed regions and
  narrower highlights in some cases. Re-wrapped code (a changed line count) couldn't be shown as
  unchanged at all: Monaco needs both sides to have the same lines between changes.
- **Cost:** +57 MB installed on every platform for a mode most diffs don't need, a second binary
  with its own patches to re-check on every difftastic upgrade, a Monaco internal to keep working,
  and a staging remap that had to agree with what's drawn.

## If it comes back

- Start from Monaco's diff and refine it; don't take alignment from difftastic.
- Measure on real repositories' history first (comments, renames, small edits, re-indents), not on
  examples built to show the feature.
- Consider cheaper ways to the main win: ignoring indentation changes inside a changed region needs
  no parser at all.
- Look for a structural differ with a library API, or tree-sitter grammars loaded on demand, rather
  than every grammar in one binary.
