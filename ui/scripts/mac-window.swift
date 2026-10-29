// The window and input side of mac-titlebar.mjs, on a macOS runner: a process's windows (as the
// window server has them), its traffic lights (through Accessibility, when this process may use
// it), and real mouse and key events posted to the system, as a person's would be.
//
//   swiftc -O -o mac-window mac-window.swift
//   mac-window windows <pid>              JSON: the pid's normal windows, on screen or not
//   mac-window screen                     JSON: the main screen's frame and visible frame (top-left origin)
//   mac-window lights <pid>               JSON: Accessibility trust, and the front window's buttons
//   mac-window fullscreen <pid> <0|1>     sets the front window's AXFullScreen
//   mac-window size <pid> <w> <h>         sets the front window's AXSize
//   mac-window menu <pid> <title>         JSON: the menu bar's menu <title>, its items' titles, keys
//                                         and modifiers (Accessibility's AXMenuItemCmd* values)
//   mac-window move <x> <y>
//   mac-window drag <x1> <y1> <x2> <y2>   press, move a little, wait, then on to x2,y2 and release
//   mac-window click <x> <y> [<count>]    a click, or a double click with <count> 2
//   mac-window key <keycode> <cmd|ctrl|opt|shift>...
//
// Points are global display coordinates, top-left origin, as CGEvent and the window list use.
import AppKit
import ApplicationServices
import CoreGraphics
import Foundation

func emit(_ value: Any) {
  let data = try! JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
  print(String(data: data, encoding: .utf8)!)
}

func rect(_ r: CGRect) -> [String: Double] {
  ["x": Double(r.origin.x), "y": Double(r.origin.y), "w": Double(r.size.width), "h": Double(r.size.height)]
}

func post(_ type: CGEventType, _ p: CGPoint, clicks: Int64 = 1) {
  let event = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: p, mouseButton: .left)!
  event.setIntegerValueField(.mouseEventClickState, value: clicks)
  event.post(tap: .cghidEventTap)
}

func pause(_ seconds: Double) { usleep(useconds_t(seconds * 1_000_000)) }

func axWindow(_ pid: pid_t) -> AXUIElement? {
  let app = AXUIElementCreateApplication(pid)
  var value: CFTypeRef?
  if AXUIElementCopyAttributeValue(app, kAXFocusedWindowAttribute as CFString, &value) == .success { return (value as! AXUIElement) }
  if AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &value) == .success, let all = value as? [AXUIElement], let first = all.first { return first }
  return nil
}

func axFrame(_ element: AXUIElement) -> [String: Double]? {
  var position: CFTypeRef?, size: CFTypeRef?
  guard AXUIElementCopyAttributeValue(element, kAXPositionAttribute as CFString, &position) == .success,
    AXUIElementCopyAttributeValue(element, kAXSizeAttribute as CFString, &size) == .success else { return nil }
  var p = CGPoint.zero, s = CGSize.zero
  AXValueGetValue(position as! AXValue, .cgPoint, &p)
  AXValueGetValue(size as! AXValue, .cgSize, &s)
  return rect(CGRect(origin: p, size: s))
}

func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
  var value: CFTypeRef?
  return AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success ? value : nil
}

func children(_ element: AXUIElement) -> [AXUIElement] {
  (attribute(element, kAXChildrenAttribute) as? [AXUIElement]) ?? []
}

let args = CommandLine.arguments.dropFirst().map { $0 }
let num = { (i: Int) -> Double in Double(args[i])! }

switch args.first ?? "" {
case "windows":
  let pid = Int(args[1])!
  let list = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as! [[String: Any]]
  emit(list.filter { ($0[kCGWindowOwnerPID as String] as? Int) == pid && ($0[kCGWindowLayer as String] as? Int) == 0 }.map { w -> [String: Any] in
    let bounds = CGRect(dictionaryRepresentation: w[kCGWindowBounds as String] as! CFDictionary)!
    return ["bounds": rect(bounds), "onscreen": (w[kCGWindowIsOnscreen as String] as? Bool) ?? false, "title": (w[kCGWindowName as String] as? String) ?? ""]
  })
case "screen":
  let screen = NSScreen.main!
  let height = screen.frame.height
  // AppKit's frames have a bottom-left origin; flip them to the top-left one.
  let flip = { (r: NSRect) in CGRect(x: r.minX, y: height - r.maxY, width: r.width, height: r.height) }
  emit(["frame": rect(flip(screen.frame)), "visible": rect(flip(screen.visibleFrame)), "scale": Double(screen.backingScaleFactor)])
case "lights":
  let pid = pid_t(Int32(args[1])!)
  var out: [String: Any] = ["trusted": AXIsProcessTrusted()]
  if let window = axWindow(pid) {
    out["window"] = axFrame(window)
    for (name, attribute) in [("close", kAXCloseButtonAttribute), ("minimize", kAXMinimizeButtonAttribute), ("zoom", kAXZoomButtonAttribute)] {
      var button: CFTypeRef?
      if AXUIElementCopyAttributeValue(window, attribute as CFString, &button) == .success { out[name] = axFrame(button as! AXUIElement) }
    }
  }
  emit(out)
case "fullscreen":
  let pid = pid_t(Int32(args[1])!)
  guard let window = axWindow(pid) else { emit(["ok": false]); exit(0) }
  let set = AXUIElementSetAttributeValue(window, "AXFullScreen" as CFString, (args[2] == "1" ? kCFBooleanTrue : kCFBooleanFalse)!)
  emit(["ok": set == .success])
case "size":
  let pid = pid_t(Int32(args[1])!)
  guard let window = axWindow(pid) else { emit(["ok": false]); exit(0) }
  var size = CGSize(width: num(2), height: num(3))
  let set = AXUIElementSetAttributeValue(window, kAXSizeAttribute as CFString, AXValueCreate(.cgSize, &size)!)
  emit(["ok": set == .success])
case "menu":
  let app = AXUIElementCreateApplication(pid_t(Int32(args[1])!))
  guard let bar = attribute(app, kAXMenuBarAttribute) else { emit(["trusted": AXIsProcessTrusted(), "found": false]); exit(0) }
  let titles = children(bar as! AXUIElement).map { (attribute($0, kAXTitleAttribute) as? String) ?? "" }
  guard let item = children(bar as! AXUIElement).first(where: { (attribute($0, kAXTitleAttribute) as? String) == args[2] }),
    let menu = children(item).first else { emit(["bar": titles, "found": false]); exit(0) }
  emit(["bar": titles, "found": true, "items": children(menu).map { i -> [String: Any] in
    ["title": (attribute(i, kAXTitleAttribute) as? String) ?? "", "enabled": (attribute(i, kAXEnabledAttribute) as? Bool) ?? false,
     "key": (attribute(i, kAXMenuItemCmdCharAttribute) as? String) ?? "", "modifiers": (attribute(i, kAXMenuItemCmdModifiersAttribute) as? Int) ?? -1]
  }])
case "move":
  post(.mouseMoved, CGPoint(x: num(1), y: num(2)))
case "drag":
  let (from, to) = (CGPoint(x: num(1), y: num(2)), CGPoint(x: num(3), y: num(4)))
  post(.mouseMoved, from)
  pause(0.2)
  post(.leftMouseDown, from)
  pause(0.15)
  // A few px first, as a hand would: the page asks for the move once the pointer has gone 4 px,
  // and the request takes a moment to come back down to the window.
  for i in 1...3 { post(.leftMouseDragged, CGPoint(x: from.x + Double(i * 3), y: from.y + Double(i * 2))); pause(0.03) }
  pause(0.4)
  let steps = 20
  for i in 1...steps {
    let t = Double(i) / Double(steps)
    post(.leftMouseDragged, CGPoint(x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t))
    pause(0.03)
  }
  pause(0.2)
  post(.leftMouseUp, to)
case "click":
  let p = CGPoint(x: num(1), y: num(2))
  let count = args.count > 3 ? Int64(args[3])! : 1
  post(.mouseMoved, p)
  pause(0.15)
  for click in 1...count {
    post(.leftMouseDown, p, clicks: click)
    pause(0.05)
    post(.leftMouseUp, p, clicks: click)
    pause(0.08)
  }
case "key":
  let code = CGKeyCode(UInt16(args[1])!)
  var flags = CGEventFlags()
  for name in args.dropFirst(2) {
    switch name {
    case "cmd": flags.insert(.maskCommand)
    case "ctrl": flags.insert(.maskControl)
    case "opt": flags.insert(.maskAlternate)
    case "shift": flags.insert(.maskShift)
    default: break
    }
  }
  for down in [true, false] {
    let event = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: down)!
    event.flags = flags
    event.post(tap: .cghidEventTap)
    pause(0.05)
  }
default:
  FileHandle.standardError.write("usage: mac-window windows|screen|lights|fullscreen|size|menu|move|drag|click|key …\n".data(using: .utf8)!)
  exit(2)
}
