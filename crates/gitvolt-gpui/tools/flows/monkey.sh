#!/usr/bin/env bash
# Random input on a fixture until the time's up (MONKEY_SECS, MONKEY_SEED): any panic in the log, a hang or a crash fails it. What it
# did is in tmp/flows/monkey-<seed>.log, to go through by hand. The app can only reach the fixture and its isolated home (realinput.sh).
source "$(dirname "$0")/lib.sh"
clean_app
SEED=${MONKEY_SEED:-$RANDOM}; SECS=${MONKEY_SECS:-120}
echo "seed $SEED, $SECS s"
fresh() {
  M=$(small_repo monkey)
  g "$M" branch -q wip/a HEAD~4; g "$M" tag -a -m 'A tag' v2.0.0 HEAD~1
  for i in 1 2 3; do echo "work $i" >>"$M/src/main.txt"; done; echo new >"$M/new.txt"; printf 'x' >"$M/stage.bin"; g "$M" add stage.bin
  N=$(long_repo monkey-long 300)
}
fresh
LOG=$TMPD/monkey-$SEED.log
up "$M"; pause 1; launch_app "$N"; pause 1
python3 - "$SEED" "$SECS" "$LOG" "$M" "$N" <<'PY'
import json, os, random, signal, socket, subprocess, sys, time
seed, secs, log, m, n = int(sys.argv[1]), float(sys.argv[2]), sys.argv[3], sys.argv[4], sys.argv[5]
rnd = random.Random(seed)
ctl = os.environ["REALINPUT_RUN"] + "/realinput.ctl"
pipe = os.environ["GITVOLT_DEBUG_PIPE"]
out = open(log, "w")

def ri(*a):
    import shlex
    c = socket.socket(socket.AF_UNIX); c.connect(ctl); c.sendall((shlex.join(a) + "\n").encode()); c.recv(100); c.close()

def pid():
    return int(open(os.environ["REALINPUT_PIDFILE"]).read().strip() or 0)

def state():
    try:
        os.unlink(pipe + ".out")
    except OSError:
        pass
    fd = os.open(pipe, os.O_WRONLY | os.O_NONBLOCK)
    os.write(fd, b"x\n"); os.close(fd)
    for _ in range(300):
        if os.path.exists(pipe + ".out"):
            return json.load(open(pipe + ".out"))
        time.sleep(0.01)
    return None

KEYS = ["Escape", "Escape", "Return", "Tab", "Shift+Tab", "Up", "Down", "Left", "Right", "Home", "End", "PageUp", "PageDown", "Space", "Delete", "BackSpace" if False else "Backspace",
        "Control+p", "Control+f", "Control+comma", "Control+slash", "Control+b", "Control+z", "Control+Shift+z", "Control+s", "Control+a", "Control+c", "Control+v", "Control+w", "Control+Shift+t",
        "Control+1", "Control+2", "Control+o", "Control+Shift+g", "Control+Shift+n", "Control+Alt+s", "Control+Alt+p", "Control+Shift+h", "Control+Shift+b", "F7", "Shift+F7", "F8", "Control+Shift+1", "Control+Shift+3",
        "Control+equal", "Control+minus", "Control+0", "Alt+1", "Alt+2", "Alt+3", "Control+Alt+g", "Control+Alt+f", "Control+Return", "Control+Shift+s", "a", "s", "d", "f", "p", "r", "e", "x", "1"]
start = time.time(); count = 0; restarts = 0
w, h = 1600, 900
st = None
while time.time() - start < secs:
    p = pid()
    alive = p and os.path.exists(f"/proc/{p}") and "Z" not in open(f"/proc/{p}/status").read().split("State:")[1][:4]
    if not alive:
        # The close button or Ctrl+W on the last tab: a normal end, come back.
        restarts += 1
        print(f"# restart {restarts}", file=out, flush=True)
        subprocess.Popen(["bash", "-c", 'start_app "$1"; echo "$APP_PID" >"$REALINPUT_PIDFILE"; wait', "x", rnd.choice([m, n])], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=False)
        time.sleep(1.5)
        continue
    count += 1
    if count % 5 == 1:
        st = state()
        if st is None:
            print(f"HUNG at action {count}", flush=True); print("HUNG", file=out, flush=True); sys.exit(4)
        w, h = int(st["window"]["w"]), int(st["window"]["h"])
    r = rnd.random()
    target = None
    anchors = list(st["anchors"].values()) if st else []
    menu = st.get("menuRows") or st.get("pickerRows") if st else None
    if menu and r < 0.5:
        row = rnd.choice(menu)
        # What closes the window every time would end the run early.
        if not any(k in row["label"] for k in ("Quit", "Close")):
            target = (row["x"], row["y"])
    if target is None and anchors and rnd.random() < 0.6:
        b = rnd.choice(anchors); target = (b[0] + b[2] / 2, b[1] + b[3] / 2)
    if target is None:
        target = (rnd.randrange(1, max(2, w - 1)), rnd.randrange(30, max(31, h - 1)))
    x, y = max(1, min(w - 2, target[0])), max(1, min(h - 2, target[1]))
    a = rnd.random()
    if a < 0.32:
        act = ["move", str(x), str(y), "--", "click"]
    elif a < 0.42:
        act = ["move", str(x), str(y), "--", "click", "right"]
    elif a < 0.50:
        act = ["move", str(x), str(y), "--", "click", "--", "click"]
    elif a < 0.60:
        act = ["move", str(x), str(y)]
    elif a < 0.70:
        act = ["move", str(x), str(y), "--", "down", "--", "move", str(x + rnd.randint(-200, 200)), str(y + rnd.randint(-120, 120)), "--", "wait", "30", "--", "up"]
    elif a < 0.78:
        act = ["move", str(x), str(y), "--", "wheel", rnd.choice(["up", "down"]), str(rnd.randint(1, 8))]
    elif a < 0.84:
        act = ["type", rnd.choice(["abc", "x y", "main", "feature", "Change", "123", ">", "/", "@"])]
    else:
        act = ["key", rnd.choice(KEYS)]
    print(" ".join(act), file=out, flush=True)
    ri(*act)
    # Quick clicks and slower ones: what's loading gets caught by the next.
    if rnd.random() < 0.3:
        time.sleep(rnd.choice([0.05, 0.2, 0.5]))
    if count % 40 == 0:
        print(f"{int(time.time() - start)} s: {count} actions, {restarts} restarts", flush=True)
print(f"{count} actions, {restarts} restarts", flush=True)
PY
rc=$?
check_sh "the run ended by its clock ($rc)" "[ $rc = 0 ]"
APP_PID=$(cat "$REALINPUT_PIDFILE"); stop_app
FLOW_NAME=monkey-$SEED finish
