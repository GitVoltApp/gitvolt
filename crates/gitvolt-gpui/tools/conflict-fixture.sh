#!/usr/bin/env bash
# Throwaway repositories for the merge tool and the interactive rebase, under target/bench/work: fictional
# authors, no remote. usage: conflict-fixture.sh merge|rebase|irebase [dir]
#   merge:   stopped in a merge of feature/parser into main: two text conflicts, a binary, a delete/modify.
#   rebase:  stopped in a rebase of topic onto main, one conflicted file, two more commits to replay.
#   irebase: clean; main with six commits above base-tag, chips feature/a and feature/b on two of them;
#            the last two both change shared.txt, so swapping them conflicts.
set -eu
kind=$1
dir=${2:-$(pwd)/target/bench/work/$kind-fixture}
rm -rf "$dir"
mkdir -p "$dir"
cd "$dir"
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
git init -q -b main
git config user.name 'Ada Fixture'
git config user.email ada@example.com
lines() { python3 - "$@" <<'PY'
import sys
name, a, b = sys.argv[1], sys.argv[2], sys.argv[3]
out = ['// %s' % name, 'use std::collections::HashMap;', '']
for i in range(1, 121):
    if i == 40:
        out += ['fn region_one(input: &str) -> usize {', '    let n = input.len(); // %s' % a, '    n + 1', '}']
    elif i == 90:
        out += ['fn region_two(map: &HashMap<String, u32>) -> u32 {', '    map.values().sum::<u32>() // %s' % b, '}']
    else:
        out.append('fn helper_%03d() -> u32 { %d }' % (i, i))
print('\n'.join(out))
PY
}
case "$kind" in
  merge)
    lines src/parser.rs base base > parser.rs
    printf 'alpha\nbeta\ngamma\ndelta\n' > notes.txt
    printf 'keep me\nsecond\n' > gone.txt
    head -c 512 /dev/zero | tr '\0' 'x' > blob.bin; printf '\0' >> blob.bin
    git add . && git commit -qm 'Base'
    git checkout -q -b feature/parser
    lines src/parser.rs incoming incoming > parser.rs
    printf 'alpha\nbeta from feature\nsecond feature line\ngamma\ndelta\n' > notes.txt
    printf 'keep me\nsecond changed\n' > gone.txt
    printf 'feature\0' > blob.bin
    git add . && git commit -qm 'Rework the parser'
    git checkout -q main
    lines src/parser.rs current current > parser.rs
    printf 'alpha\nbeta from main\ngamma\ndelta\n' > notes.txt
    git rm -q gone.txt
    printf 'main\0' > blob.bin
    git add . && git commit -qm 'Tune the parser on main'
    git merge --no-edit feature/parser >/dev/null 2>&1 || true
    ;;
  rebase)
    lines src/parser.rs base base > parser.rs
    git add . && git commit -qm 'Base'
    git checkout -q -b topic
    lines src/parser.rs topic base > parser.rs
    git add . && git commit -qm 'Topic edits region one'
    printf 'one\n' > one.txt && git add . && git commit -qm 'Topic adds one'
    printf 'two\n' > two.txt && git add . && git commit -qm 'Topic adds two'
    git checkout -q main
    lines src/parser.rs main base > parser.rs
    git add . && git commit -qm 'Main edits region one'
    git checkout -q topic
    git rebase main >/dev/null 2>&1 || true
    ;;
  irebase)
    printf 'base\n' > base.txt && printf 'shared 0\n' > shared.txt && git add . && git commit -qm 'Base of the plan'
    git tag base-tag
    git branch trunk
    for i in 1 2 3 4 5 6; do
      printf 'line %s\n' "$i" > "file$i.txt"
      if [ "$i" -ge 5 ]; then printf 'shared %s\n' "$i" > shared.txt; fi
      git add . && GIT_AUTHOR_NAME="Bob Example" GIT_AUTHOR_EMAIL=bob@example.com git commit -qm "Commit number $i" -m "Body of commit $i."
      if [ "$i" = 2 ]; then git branch feature/a; fi
      if [ "$i" = 4 ]; then git branch feature/b; fi
    done
    ;;
esac
echo "$dir"
