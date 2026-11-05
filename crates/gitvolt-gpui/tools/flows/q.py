#!/usr/bin/env python3
"""What `lib.sh`'s `q` evaluates: an expression over `d`, the app's state, and these helpers. Prints `x y` for a point, or nothing."""
import json, sys

d = json.load(open(sys.argv[2]))


# What the app calls (0,0) is the window's, the input's is the screen's: they part when a window's been moved on Xwayland.
OX, OY = d.get("window", {}).get("x", 0), d.get("window", {}).get("y", 0)


def centre(b):
    return f"{round(b[0] + b[2] / 2 + OX)} {round(b[1] + b[3] / 2 + OY)}"


def at(key):
    for k, b in d["anchors"].items():
        if k == key:
            return centre(b)
    for k, b in d["anchors"].items():
        if k.endswith(key):
            return centre(b)
    return ""


def row_at(text):
    for r in d["menuRows"][::-1] + d["pickerRows"]:
        if text in r["label"]:
            return f"{round(r['x'] + OX)} {round(r['y'] + OY)}"
    return ""


def rowat(text):
    for r in d["graphRows"]:
        if text in r["summary"]:
            b = d["anchors"].get("tip:msg:" + str(r["id"]))
            if b:
                return f"{round(b[0] + 60 + OX)} {round(b[1] + b[3] / 2 + OY)}"
    return ""


print(eval(sys.argv[1]))
