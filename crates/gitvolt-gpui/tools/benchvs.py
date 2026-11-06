#!/usr/bin/env python3
"""One run of one build in a nested compositor, by what a user sees and what /proc says, the same for the GPUI build and a release
of the CEF one (which has no debug port and no state probe). Run by `benchvs.sh`, inside `realinput.sh`.
usage: benchvs.py <label> <repo> <file-x> <file-y> <out.jsonl> <shots-dir>"""
import json, os, subprocess, sys, time
import xml.etree.ElementTree as ET

import dbus
from PIL import Image

label, repo, file_x, file_y, out, shots = sys.argv[1:7]
root = os.environ["REALINPUT_ROOT"]
tools = os.path.dirname(os.path.abspath(__file__))
native = bool(os.environ.get("REALINPUT_NATIVE"))
bus = dbus.bus.BusConnection(os.environ["GITVOLT_NESTED_BUS"])
kwin = dbus.Interface(bus.get_object("org.kde.KWin", "/org/kde/KWin/ScreenShot2"), "org.kde.KWin.ScreenShot2")
tick = os.sysconf("SC_CLK_TCK")
binary = os.path.realpath(os.environ["GITVOLT_GPUI_BIN"])
GRAPH = (300, 140, 800, 500)
PANE = (60, 200, 1040, 600)


def grab(area):
    r, w = os.pipe()
    meta = kwin.CaptureArea(area[0], area[1], area[2], area[3], {"native-resolution": True}, dbus.types.UnixFd(w), timeout=5)
    os.close(w)
    chunks = []
    while chunk := os.read(r, 1 << 20):
        chunks.append(chunk)
    os.close(r)
    return b"".join(chunks), int(meta["width"]), int(meta["height"]), int(meta["stride"])


def image(area):
    data, w, h, stride = grab(area)
    return Image.frombuffer("RGBA", (w, h), data, "raw", "BGRA", stride, 1).convert("RGB")


def ri(*args):
    subprocess.run([sys.executable, f"{tools}/realinput.py", "--send", *args], check=True, stdout=subprocess.DEVNULL)


def cgroup(pid):
    try:
        return open(f"/proc/{pid}/cgroup").read()
    except OSError:
        return None


def tree(top):
    """The app and everything under it, and (Chromium's helpers can leave the tree) what runs its binary in this scope: the same
    binary outside it is the user's own instance."""
    parents, cmd = {}, {}
    mine = cgroup(top)
    for p in os.listdir("/proc"):
        if not p.isdigit():
            continue
        try:
            stat = open(f"/proc/{p}/stat").read()
            parents[int(p)] = int(stat[stat.rindex(")") + 2 :].split()[1])
            cmd[int(p)] = open(f"/proc/{p}/cmdline", "rb").read().split(b"\0")[0].decode(errors="replace")
        except OSError:
            continue
    found = {top}
    grew = True
    while grew:
        grew = False
        for p, parent in parents.items():
            if parent in found and p not in found:
                found.add(p)
                grew = True
    found |= {p for p, c in cmd.items() if c == binary and cgroup(p) == mine}
    return found


def cpu(pids):
    total = 0
    for p in pids:
        try:
            stat = open(f"/proc/{p}/stat").read()
            fields = stat[stat.rindex(")") + 2 :].split()
            total += int(fields[11]) + int(fields[12])
        except OSError:
            continue
    return total / tick


def memory(pids):
    sums = {"anon_mb": 0.0, "rss_mb": 0.0, "pss_mb": 0.0, "pss_unreadable": 0, "processes": 0}
    for p in pids:
        try:
            status = open(f"/proc/{p}/status").read()
        except OSError:
            continue
        values = {line.split(":")[0]: line.split()[1] for line in status.splitlines() if line.startswith(("VmRSS", "RssAnon"))}
        if "VmRSS" not in values:
            continue
        sums["processes"] += 1
        sums["rss_mb"] += int(values["VmRSS"]) / 1024
        sums["anon_mb"] += int(values.get("RssAnon", 0)) / 1024
        try:
            for line in open(f"/proc/{p}/smaps_rollup"):
                if line.startswith("Pss:"):
                    sums["pss_mb"] += int(line.split()[1]) / 1024
        except OSError:
            sums["pss_unreadable"] += 1
    return {k: round(v, 1) for k, v in sums.items()}


def gpu_mb(pids):
    try:
        xml = subprocess.run(["nvidia-smi", "-q", "-x"], capture_output=True, text=True, timeout=10).stdout
        total, seen = 0.0, False
        for proc in ET.fromstring(xml).iter("process_info"):
            if int(proc.findtext("pid")) in pids:
                total += float(proc.findtext("used_memory").split()[0])
                seen = True
        return round(total, 1) if seen else None
    except Exception:
        return None


def spent(top, seconds):
    pids = tree(top)
    before, t = cpu(pids), time.monotonic()
    time.sleep(seconds)
    pids |= tree(top)
    return round((cpu(pids) - before) / (time.monotonic() - t) * 100, 2)


os.makedirs(shots, exist_ok=True)
result = {"label": label, "native": native, "load": os.getloadavg()[0], "available_mb": int(open("/proc/meminfo").read().split("MemAvailable:")[1].split()[0]) // 1024}
log = open(f"{root}/app.log", "a")
t0 = time.monotonic()
app = subprocess.Popen([f"{root}/launch-app", repo], stdout=log, stderr=log)
try:
    # The first picture with the graph's rows in it: text and lanes make many colours.
    while time.monotonic() - t0 < 30:
        colours = image(GRAPH).getcolors(maxcolors=4096)
        if colours is None or len(colours) >= 40:
            result["start_ms"] = round((time.monotonic() - t0) * 1000)
            break
        time.sleep(0.01)
    time.sleep(5)
    image((0, 0, 1600, 900)).save(f"{shots}/{label}-graph.png")
    result["idle_pct"] = spent(app.pid, 20)
    pids = tree(app.pid)
    result["after_settling"] = memory(pids)
    result["gpu_mb"] = gpu_mb(pids)

    if not native:
        ri("move", "700", "400", "--", "wait", "100")
        pids = tree(app.pid)
        before, t = cpu(pids), time.monotonic()
        while time.monotonic() - t < 8:
            ri("wheel", "down", "2", "--", "wait", "100")
        result["scroll_pct"] = round((cpu(pids | tree(app.pid)) - before) / (time.monotonic() - t) * 100, 2)
        image((0, 0, 1600, 900)).save(f"{shots}/{label}-scrolled.png")
        ri("wheel", "up", "100", "--", "wait", "600")

    # The working copy's row, then its one big file: until the pane stops changing.
    ri("move", "700", "147", "--", "wait", "60", "--", "click", "--", "wait", "1500")
    ri("move", file_x, file_y, "--", "wait", "300")
    ri("click")
    t = time.monotonic()
    last, changed = grab(PANE)[0], t
    while time.monotonic() - changed < 2.0 and time.monotonic() - t < 10:
        now = grab(PANE)[0]
        if now != last:
            last, changed = now, time.monotonic()
    result["diff_open_ms"] = round((changed - t) * 1000)
    image((0, 0, 1600, 900)).save(f"{shots}/{label}-diff.png")
    time.sleep(2)
    result["idle_diff_pct"] = spent(app.pid, 10)
    result["with_diff"] = memory(tree(app.pid))
finally:
    for p in sorted(tree(app.pid) - {app.pid}):
        try:
            os.kill(p, 15)
        except OSError:
            pass
    app.terminate()
    try:
        app.wait(5)
    except subprocess.TimeoutExpired:
        app.kill()
with open(out, "a") as f:
    f.write(json.dumps(result) + "\n")
print(json.dumps(result))
