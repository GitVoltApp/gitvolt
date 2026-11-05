#!/usr/bin/env python3
"""A picture of a nested compositor (`tools/realinput.sh`), on Xwayland and on Wayland alike: KWin's own screenshot call, on the
nested compositor's private bus. Refuses without that bus, so it can't see the desktop.
usage: wshot.py out.png [x y w h]"""
import os, struct, sys
import dbus
from PIL import Image

addr = os.environ.get("GITVOLT_NESTED_BUS")
if not addr:
    sys.exit("refusing: no GITVOLT_NESTED_BUS; this never looks at the real desktop")
bus = dbus.bus.BusConnection(addr)
shots = dbus.Interface(bus.get_object("org.kde.KWin", "/org/kde/KWin/ScreenShot2"), "org.kde.KWin.ScreenShot2")
area = [int(v) for v in sys.argv[2:6]] or [0, 0, int(os.environ.get("NESTED_W", 1920)), int(os.environ.get("NESTED_H", 1200))]
r, w = os.pipe()
meta = shots.CaptureArea(area[0], area[1], area[2], area[3], {"native-resolution": True}, dbus.types.UnixFd(w), timeout=20)
os.close(w)
data = b""
while True:
    chunk = os.read(r, 1 << 20)
    if not chunk:
        break
    data += chunk
width, height, stride, fmt = int(meta["width"]), int(meta["height"]), int(meta["stride"]), int(meta["format"])
# QImage's formats: 4 RGB32, 5 ARGB32, 6 ARGB32_Premultiplied are BGRA in memory; 16-18 RGBX/RGBA.
mode = "BGRA" if fmt in (4, 5, 6) else "RGBA"
img = Image.frombuffer("RGBA", (width, height), data, "raw", mode, stride, 1).convert("RGB")
img.save(sys.argv[1])
