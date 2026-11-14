#!/usr/bin/env bash
# native: yes
# Rendered Markdown by mouse, diagrams, a live theme switch with a diff open, a details toggle, a <video> box.
source "$(dirname "$0")/lib.sh"
clean_app
R=$WORK/flows/render; rm -rf "$R"; mkdir -p "$R"
g "$R" init -q -b main; g "$R" config user.name 'Ada Fixture'; g "$R" config user.email ada@example.com
cat >"$R/page.md" <<'MD'
# Page

<details><summary>More here</summary>

The hidden words.

</details>

<video src="clip.mp4" controls></video>

[A link](https://www.example.com/fictional)

```mermaid
flowchart TD
  A --> B
```

| a | b |
|---|---|
| 1 | 2 |
MD
g "$R" add .; g "$R" commit -qm "Page"; echo "new" >>"$R/page.md"
up "$R"; until_q "d['rows'] and d['rows'] >= 1" 5; pause 0.4
clickxy 700 147; until_q "d['wip'] and d['wip']['unstaged']" 5
clickon "tip:page.md"; until_q "d['md'] and d['md']['ready']" 6; pause 1.2
shot render-0
H0=$(q "d['md']['total']")
echo "the details toggle"
ri move 200 240 -- wait 100 -- click -- wait 300
q "d['md']['total'], d['toast']"; shot render-1
check_sh "it opened: taller ($H0)" "[ \"\$(q \"d['md']['total'] > $H0\")\" = True ]"
echo "the theme, live, with diagrams and a diff"
key Control+comma; pause 0.4; clickxy 300 173; pause 0.5; shot render-theme
q "d['settings']['page'], d['settings']['themeShown']"
clickon "theme-card:light"; pause 0.8
check "the light theme, live" "d['settings']['themeShown'] == 'light'"
shot render-light-settings
key Escape; pause 0.8
check "still on the file" "d['md'] and d['md']['ready']"
shot render-light
key Control+comma; pause 0.4; clickon "theme-card:default-dark"; pause 0.6; key Escape; pause 0.8
check "and back" "d['settings']['themeShown'] == 'default-dark'"
shot render-dark
finish
