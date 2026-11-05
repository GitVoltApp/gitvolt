#!/usr/bin/env bash
# A throwaway repository for the rendered views: target/bench/work/md-fixture, made anew. Fictional
# authors and content; local pictures only. Two commits: every file's old version, then its new one.
# Also a 5000-line Markdown file (big.md) and generated PDFs, a font, a WAV and short videos.
set -eu
root=$(pwd)
dir="$root/target/bench/work/md-fixture"
rm -rf "$dir"
mkdir -p "$dir/docs/img" "$dir/rich"
cd "$dir"
git init -q -b main
git config user.name 'Ada Fixture'
git config user.email ada@example.com
python3 - <<'PY'
from PIL import Image, ImageDraw
def pic(path, w, h, a, b, text):
    im = Image.new('RGB', (w, h), a)
    d = ImageDraw.Draw(im)
    for i in range(0, w, 16):
        d.line([(i, 0), (w - i, h)], fill=b)
    d.text((8, 8), text, fill=(255, 255, 255))
    im.save(path)
pic('docs/img/harbour.png', 320, 120, (32, 80, 128), (64, 128, 192), 'harbour')
pic('docs/img/wide.png', 2400, 300, (120, 60, 30), (200, 140, 60), 'wide')
pic('docs/img/small.png', 64, 64, (30, 120, 80), (90, 200, 140), 'small')
PY
cat > docs/lighthouse.svg <<'S'
<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" viewBox="0 0 96 96"><rect width="96" height="96" fill="#26547c"/><circle cx="48" cy="48" r="30" fill="#ffd166"/></svg>
S
cat > README.md <<'MD'
---
title: Harbour guide
version: 3
draft: false
tags: [boats, tides]
owner:
  name: Ada Fixture
---

# Harbour guide

A guide to the **fictional** harbour of *Port Example*, with `inline code`, ~~old words~~ and a [link to the tides](docs/tides.md#spring-tides). See https://example.com/harbour and mail ada@example.com. Press <kbd>Ctrl</kbd>+<kbd>K</kbd>, H<sub>2</sub>O and E=mc<sup>2</sup>. Emoji :rocket: and :+1: stay as glyphs; :notanemoji: stays text.

## Lists

- First item with a long line that should wrap around at the end of the measure so that the second line can be compared with the reference image, word by word and pixel by pixel.
- Second item
  - Nested item
    - Deeper item
  - Nested again

    A second paragraph in a nested item.
- Third item

1. One
2. Two
   1. Two point one
   2. Two point two
3. Three

5. Starts at five
6. And goes on

### Task list

- [x] Moor the boat
- [ ] Check the tide
- [x] Lock the gate
  - [ ] Nested task

## Tables

| Left | Centre | Right |
|:-----|:------:|------:|
| a | b | c |
| longer left cell | centred text | 1,234.50 |
| `code` | **bold** | [link](#lists) |
| one | two | three |

| Tide | Time | Note |
|---|---|---|
| High | 06:12 | A very long note about the morning tide that goes on and on, so that the column has to wrap inside the table and the table doesn't scroll sideways until it has run out of room |
| Low | 12:30 | Short |

## Code

Some code, in a fence:

```rust
fn main() {
    // A comment
    let tide = "high";
    println!("{tide}");
}
```

```
plain text without a language, and a very long line that runs off the right edge of the measure so the block has to scroll sideways on its own, without the page moving at all
```

```ts
export const harbour = { depth: 12, name: 'Port Example' } as const;
```

    indented code block
      with two lines

## Quotes and alerts

> A quote with **bold** text.
>
> And a second paragraph.
>
> > A nested quote.

> [!NOTE]
> A GitHub-style alert, shown as its Markdown says.

> [!WARNING]
> Mind the tide.

## Footnotes

The first note[^one] and the second[^two], and the first again[^one].

[^one]: The first note's text.
[^two]: The second note, with `code` and [a link](https://example.org/notes).

## Images and links

![Harbour at dusk](docs/img/harbour.png)

A picture in text ![small](docs/img/small.png) with words around it, and a linked one: [![small](docs/img/small.png)](https://example.com/small).

![A wide picture](docs/img/wide.png)

![An SVG](docs/lighthouse.svg)

![Remote picture](https://images.example.net/quay.png)

![Missing picture](docs/img/nothing.png)

<img src="docs/img/small.png" width="32" height="32" alt="sized">

Reference style: [the tides][tides] and #12, a commit abc1234def and @someone in text.

[tides]: docs/tides.md "Tide tables"

---

## HTML

<details>
<summary>More about the quay</summary>

Hidden **text** inside the details, with a list:

- one
- two

</details>

<details open>
<summary>Open by default</summary>

Shown from the start.

</details>

<div align="center">A div with <em>inline</em> HTML.</div>

<dl><dt>Term</dt><dd>Its definition.</dd></dl>

<script>alert('never')</script>

<a name="anchor-target"></a>

<p>Paragraph with <mark>marked</mark> and <small>small</small> and <abbr title="Harbour">Hbr</abbr> text.</p>

Heading levels:

# H1 again
## H2 again
### H3 again
#### H4 text
##### H5 text
###### H6 text

Line one\
line two with a hard break.

Last paragraph.
MD
cat > docs/tides.md <<'MD'
# Tides

Back to the [guide](../README.md).

## Spring tides

The highest tides of the month.

## Neap tides

The lowest ones.
MD
cat > CHANGES.md <<'MD'
# Changes

A first paragraph that stays the same in both versions.

A paragraph whose wording changes in the middle of the sentence, and stays around it.

## Old heading

- Kept item
- Item to change a little
- Item to remove

| Name | Value |
|---|---|
| kept | 1 |
| changed | 2 |
| gone | 3 |

```rust
fn old() {
    let a = 1;
    println!("{a}");
}
```

Removed paragraph that is only on the old side.

![Harbour](docs/img/harbour.png)

```mermaid
graph TD
  A[Quay] --> B[Gate]
```
MD
cat > flow.mmd <<'MD'
graph TD
  A[Quay] --> B{Tide high?}
  B -->|Yes| C[Sail]
  B -->|No| D[Wait]
MD
cat > seq.mmd <<'MD'
sequenceDiagram
  Ada->>Ben: Is the tide high?
  Ben-->>Ada: Not yet
MD
git add .
git -c commit.gpgsign=false commit -qm 'Old versions' --date '2026-01-01T10:00:00' 
cat > CHANGES.md <<'MD'
# Changes

A first paragraph that stays the same in both versions.

A paragraph whose phrasing changes in the middle of the sentence, and stays around it.

## New heading

- Kept item
- Item to change quite a little
- New item at the end

| Name | Value |
|---|---|
| kept | 1 |
| changed | 22 |
| fresh | 4 |

```rust
fn new() {
    let a = 2;
    println!("{a}");
}
```

Added paragraph that is only on the new side.

![Harbour](docs/img/small.png)

```mermaid
graph TD
  A[Quay] --> C[Sea]
```
MD
python3 - <<'PY'
# A generated PDF: text, shapes and a second page, with no library.
def pdf(path, pages, media=(612, 792)):
    objs = []
    def add(b):
        objs.append(b)
        return len(objs)
    font = add(b'<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>')
    font2 = add(b'<< /Type /Font /Subtype /Type1 /BaseFont /Times-Italic >>')
    kids = []
    pages_id = len(objs) + 2 * len(pages) + 1
    for i, text in enumerate(pages):
        stream = text.encode('latin-1')
        c = add(b'<< /Length %d >>\nstream\n' % len(stream) + stream + b'\nendstream')
        p = add(b'<< /Type /Page /Parent %d 0 R /MediaBox [0 0 %d %d] /Contents %d 0 R /Resources << /Font << /F1 %d 0 R /F2 %d 0 R >> >> >>' % (pages_id, media[0], media[1], c, font, font2))
        kids.append(p)
    add(b'<< /Type /Pages /Kids [' + b' '.join(b'%d 0 R' % k for k in kids) + b'] /Count %d >>' % len(kids))
    root = add(b'<< /Type /Catalog /Pages %d 0 R >>' % pages_id)
    out = b'%PDF-1.4\n'
    offs = []
    for n, o in enumerate(objs, 1):
        offs.append(len(out))
        out += b'%d 0 obj\n' % n + o + b'\nendobj\n'
    x = len(out)
    out += b'xref\n0 %d\n0000000000 65535 f \n' % (len(objs) + 1)
    for o in offs:
        out += b'%010d 00000 n \n' % o
    out += b'trailer\n<< /Size %d /Root %d 0 R >>\nstartxref\n%d\n%%%%EOF\n' % (len(objs) + 1, root, x)
    open(path, 'wb').write(out)
page1 = 'BT /F1 24 Tf 72 700 Td (Port Example tide tables) Tj ET\nBT /F2 14 Tf 72 670 Td (A fictional document, page one) Tj ET\n0.15 0.33 0.49 rg 72 500 200 120 re f\n0.9 0.3 0.2 RG 4 w 300 500 m 500 620 l S\n1 0.82 0.4 rg 420 560 m 470 560 470 610 420 610 c 370 610 370 560 420 560 c f\n0 0 0 rg BT /F1 12 Tf 72 460 Td (High water 06:12, low water 12:30, high water 18:40.) Tj ET'
page2 = 'BT /F1 18 Tf 72 700 Td (Page two: the quay) Tj ET\n0.2 0.6 0.4 rg 72 300 468 300 re f\n1 1 1 rg BT /F1 36 Tf 120 430 Td (Mind the gap) Tj ET'
page3 = 'BT /F1 18 Tf 72 700 Td (Page three) Tj ET'
pdf('rich/tides.pdf', [page1, page2, page3])
PY
python3 - <<'PY'
import wave, math, struct
w = wave.open('rich/bell.wav', 'wb')
w.setnchannels(1); w.setsampwidth(2); w.setframerate(22050)
w.writeframes(b''.join(struct.pack('<h', int(12000 * math.sin(i * 0.05) * math.exp(-i / 9000))) for i in range(22050)))
w.close()
PY
cp "$root/crates/gitvolt-gpui/assets/fonts/JetBrainsMono-Latin-400.ttf" rich/face.ttf
ffmpeg -loglevel error -y -f lavfi -i testsrc=duration=1:size=160x120:rate=10 -c:v libvpx -b:v 200k rich/clip.webm 2>/dev/null || true
ffmpeg -loglevel error -y -f lavfi -i testsrc=duration=1:size=160x120:rate=10 -c:v libx264 -pix_fmt yuv420p rich/clip.mp4 2>/dev/null || true
python3 - <<'PY'
import random
random.seed(7)
words = 'harbour tide quay gate boat mooring chart depth pier lantern rope keel anchor sail current buoy dock swell ebb flood'.split()
out = ['# Big document', '']
for i in range(1, 2000):
    if len(out) >= 5000: break
    k = i % 12
    if k == 0: out += [f'## Section {i}', '']
    elif k == 1: out += [f'### Part {i}', '']
    elif k == 5:
        out += ['```rust', f'fn tide_{i}() -> u32 {{', f'    let depth = {i};', '    depth * 2', '}', '```', '']
    elif k == 7:
        out += [f'- item {j} of {i}: ' + ' '.join(random.choices(words, k=8)) for j in range(6)] + ['']
    elif k == 9:
        out += ['| A | B | C |', '|---|---|---|'] + [f'| {j} | {" ".join(random.choices(words, k=2))} | **{j * i}** |' for j in range(4)] + ['']
    else:
        for _ in range(3):
            out.append(' '.join(random.choices(words, k=14)) + f' `code{i}` and *{random.choice(words)}*.')
        out.append('')
open('big.md', 'w').write('\n'.join(out[:5000]) + '\n')
PY
git add .
git -c commit.gpgsign=false commit -qm 'New versions' --date '2026-02-01T10:00:00'
echo "$dir"
