#!/usr/bin/env python3
"""Real input into a nested KWin (`tools/realinput.sh`), by its `org_kde_kwin_fake_input`: the compositor sends the
events to the focused window as a device's, through the platform's own code. Speaks Wayland's wire format itself.

Only for the nested compositor's socket, never the desktop's: it refuses without GITVOLT_NESTED_SOCKET.
usage: realinput.py [--serve | --send] <verb> [args]...   verbs, chained with `--`:
  move X Y | click [left|right|middle] | dclick | down [BTN] | up [BTN] | wheel up|down [N]
  scroll DX DY (a wheel's axis values, 15 a notch; DX across) | key KEY[+KEY...] | raw CODE[+CODE...] (evdev key codes, whatever
  the layout says they mean) | hold KEY | release KEY | type TEXT | wait MS
"""
import os, shlex, socket, struct, sys, time

path = os.environ.get("GITVOLT_NESTED_SOCKET")
if not path:
    sys.exit("refusing: no GITVOLT_NESTED_SOCKET; this never drives the real desktop")
# One device for a whole run: a process of its own per command would be a new device each time, which KWin takes away with
# whatever it held down, and a press from a device just added can get lost. `--serve` holds it; `--send` is a command to it.
control = os.environ.get("REALINPUT_RUN", "/nonexistent") + "/realinput.ctl"
if len(sys.argv) > 1 and sys.argv[1] == "--send":
    c = socket.socket(socket.AF_UNIX)
    c.connect(control)
    c.sendall((shlex.join(sys.argv[2:]) + "\n").encode())
    reply = c.recv(4096).decode().strip()
    sys.exit(0 if reply == "ok" else reply)
s = socket.socket(socket.AF_UNIX)
s.connect(path)


def send(obj, op, payload=b""):
    s.sendall(struct.pack("<IHH", obj, op, 8 + len(payload)) + payload)


def string(t):
    b = t.encode() + b"\0"
    return struct.pack("<I", len(b)) + b + b"\0" * (-len(b) % 4)


def fixed(v):
    return struct.pack("<i", int(v * 256))


DISPLAY, REGISTRY, CALLBACK, FAKE = 1, 2, 3, 4
send(DISPLAY, 1, struct.pack("<I", REGISTRY))
send(DISPLAY, 0, struct.pack("<I", CALLBACK))
buf, found, done = b"", None, False
while not done:
    buf += s.recv(65536)
    while len(buf) >= 8:
        obj, opsize = struct.unpack_from("<II", buf)
        op, size = opsize & 0xffff, opsize >> 16
        if len(buf) < size:
            break
        body = buf[8:size]
        buf = buf[size:]
        if obj == REGISTRY and op == 0:
            name, n = struct.unpack_from("<II", body)
            iface = body[8:8 + n - 1].decode()
            version = struct.unpack_from("<I", body, 8 + n + (-n % 4))[0]
            if iface == "org_kde_kwin_fake_input":
                found = (name, version)
        elif obj == CALLBACK:
            done = True
if not found:
    sys.exit("the compositor offers no org_kde_kwin_fake_input")
send(REGISTRY, 0, struct.pack("<I", found[0]) + string("org_kde_kwin_fake_input") + struct.pack("<II", found[1], FAKE))
send(FAKE, 0, string("gitvolt-gpui realinput") + string("testing"))
next_id = FAKE + 1


def sync():
    # A roundtrip: what was sent has reached the compositor.
    global next_id
    callback, next_id = next_id, next_id + 1
    send(DISPLAY, 0, struct.pack("<I", callback))
    data = b""
    while True:
        data += s.recv(65536)
        off = 0
        while off + 8 <= len(data):
            obj, opsize = struct.unpack_from("<II", data, off)
            size = opsize >> 16
            if off + size > len(data):
                break
            if obj == callback:
                time.sleep(0.03)
                return
            off += size
        data = data[off:]


BTN = {"left": 0x110, "right": 0x111, "middle": 0x112}
SYM = {"Control": 0xffe3, "Ctrl": 0xffe3, "Shift": 0xffe1, "Alt": 0xffe9, "Return": 0xff0d, "Enter": 0xff0d, "Escape": 0xff1b, "Esc": 0xff1b, "Tab": 0xff09, "Backspace": 0xff08, "Delete": 0xffff, "Space": 0x20, "Home": 0xff50, "End": 0xff57, "Left": 0xff51, "Up": 0xff52, "Right": 0xff53, "Down": 0xff54, "PageUp": 0xff55, "PageDown": 0xff56, "comma": 0x2c, "slash": 0x2f, "equal": 0x3d, "minus": 0x2d, "plus": 0x2b, "F10": 0xffc7}
SYM.update({f"F{i}": 0xffbe + i - 1 for i in range(1, 13)})


def keysym(k):
    return SYM[k] if k in SYM else ord(k)


def chord(spec):
    keys = spec.split("+")
    for k in keys:
        send(FAKE, 12, struct.pack("<II", keysym(k), 1))
    sync()
    for k in reversed(keys):
        send(FAKE, 12, struct.pack("<II", keysym(k), 0))
    sync()


def run(a):
    while a:
        v = a.pop(0)
        if v == "--":
            continue
        if v == "move":
            send(FAKE, 9, fixed(float(a.pop(0))) + fixed(float(a.pop(0))))
            sync()
        elif v in ("click", "down", "up", "dclick"):
            b = BTN[a.pop(0)] if a and a[0] in BTN else BTN["left"]
            for _ in range(2 if v == "dclick" else 1):
                if v != "up":
                    send(FAKE, 2, struct.pack("<II", b, 1))
                    sync()
                if v != "down":
                    send(FAKE, 2, struct.pack("<II", b, 0))
                    sync()
        elif v == "wheel":
            # Through Xwayland, 10 of the axis is a device's notch. On Wayland, what the fake device sends is what a touchpad does,
            # with no discrete notch, and the same pixels arrive as a client's continuous scroll: a real wheel's path can't run here.
            d = {"up": -10.0, "down": 10.0}[a.pop(0)]
            for _ in range(int(a.pop(0)) if a and a[0].isdigit() else 1):
                send(FAKE, 3, struct.pack("<I", 0) + fixed(d))
                sync()
        elif v == "scroll":
            dx, dy = float(a.pop(0)), float(a.pop(0))
            if dy:
                send(FAKE, 3, struct.pack("<I", 0) + fixed(dy))
            if dx:
                send(FAKE, 3, struct.pack("<I", 1) + fixed(dx))
            sync()
        elif v == "raw":
            codes = [int(c) for c in a.pop(0).split("+")]
            for c in codes:
                send(FAKE, 10, struct.pack("<II", c, 1))
            sync()
            for c in reversed(codes):
                send(FAKE, 10, struct.pack("<II", c, 0))
            sync()
        elif v in ("hold", "release"):
            send(FAKE, 12, struct.pack("<II", keysym(a.pop(0)), 1 if v == "hold" else 0))
            sync()
        elif v == "key":
            chord(a.pop(0))
        elif v == "type":
            for c in a.pop(0):
                chord("Return" if c == "\n" else c)
        elif v == "wait":
            time.sleep(int(a.pop(0)) / 1000)
        else:
            raise ValueError(f"unknown verb {v!r}")

if len(sys.argv) > 1 and sys.argv[1] == "--serve":
    server = socket.socket(socket.AF_UNIX)
    if os.path.exists(control):
        os.unlink(control)
    server.bind(control)
    server.listen(8)
    while True:
        c, _ = server.accept()
        line = b""
        while not line.endswith(b"\n"):
            chunk = c.recv(65536)
            if not chunk:
                break
            line += chunk
        try:
            run(shlex.split(line.decode()))
            c.sendall(b"ok\n")
        except Exception as e:
            c.sendall(f"{e}\n".encode())
        c.close()
else:
    run(sys.argv[1:])
