#!/usr/bin/env bash
# A throwaway repository to port and compare against: target/bench/work/fixture, made anew. Fictional
# authors; two big text files, a picture, a binary, branches, a tag, a stash and uncommitted changes.
set -eu
root=$(pwd)
dir="$root/target/bench/work/fixture"
rm -rf "$dir"
mkdir -p "$dir"
cd "$dir"
git init -q -b main
git config user.name 'Ada Fixture'
git config user.email ada@example.com
find "$root/ui/src" -name '*.ts' -not -name '*.test.ts' -not -path '*/gen/*' -not -path '*/demo/*' | sort | xargs cat | head -5000 > big.ts || true
find "$root/crates/gitvolt-core/src" -name '*.rs' | sort | xargs cat | head -5000 > big.rs || true
mkdir -p src/deep/er
printf '# Fixture\n\nSome *text* and a [link](http://example.com).\n' > README.md
printf 'fn main() {\n    println!("hi");\n}\n' > src/deep/er/main.rs
git add . && git commit -qm 'Base'
python3 - <<'PY'
for f in ['big.ts', 'big.rs']:
    lines = open(f).read().split('\n')
    out = []
    for i, l in enumerate(lines):
        if i % 97 == 40:
            out.append(l.replace('const', 'let').replace('let ', 'const ', 1) + ' // changed')
            continue
        if i % 211 == 100:
            continue
        if i % 307 == 150:
            out.append('// inserted line ' + str(i))
        out.append(l)
    open(f, 'w').write('\n'.join(out))
PY
GIT_AUTHOR_NAME='Bob Example' GIT_AUTHOR_EMAIL=bob@example.com git add . && GIT_AUTHOR_NAME='Bob Example' GIT_AUTHOR_EMAIL=bob@example.com git commit -qm 'Change both big files'
python3 - <<'PY'
import struct, zlib
def png(path, w, h, f):
    raw = b''.join(b'\0' + bytes(c for x in range(w) for c in f(x, y)) for y in range(h))
    ch = lambda t, d: struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d))
    open(path, 'wb').write(b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)) + ch(b'IDAT', zlib.compress(raw)) + ch(b'IEND', b''))
png('pic.png', 320, 200, lambda x, y: (x % 256, y % 256, 120))
open('data.bin', 'wb').write(bytes(range(256)) * 4)
PY
git add . && git commit -qm 'Add a picture and a blob'
python3 - <<'PY'
import struct, zlib
def png(path, w, h, f):
    raw = b''.join(b'\0' + bytes(c for x in range(w) for c in f(x, y)) for y in range(h))
    ch = lambda t, d: struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d))
    open(path, 'wb').write(b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)) + ch(b'IDAT', zlib.compress(raw)) + ch(b'IEND', b''))
png('pic.png', 400, 240, lambda x, y: (255 - x % 256, y % 256, 40))
d = bytearray(open('data.bin', 'rb').read()); d[100:110] = b'CHANGED!!!'; open('data.bin', 'wb').write(d)
PY
git add . && git commit -qm 'Change the picture and the blob'
git tag v1.0.0
git branch feature/one HEAD~1
git branch feature/two HEAD~2
git checkout -q -b feature/three
printf 'three\n' > three.txt && git add . && git commit -qm 'A commit on a feature branch'
git checkout -q main
git merge -q --no-ff -m "Merge branch 'feature/three'" feature/three
printf 'stashed\n' >> README.md && git stash -q
printf 'unstaged change\n' >> README.md
printf 'staged\n' > staged.txt && git add staged.txt
printf 'untracked\n' > untracked.txt
echo "$dir"
