# GitVolt's own title bar on Windows, checked on the runner's desktop with real input (SendInput,
# keybd_event): window, client and DWM frame rectangles, cursors, pixels and screenshots, into
# <Out>/results.md and <Out>/*.png. A check that fails is logged and the next one runs; exits 1
# if any failed. Run by .github/workflows/windows-titlebar.yml.
#
#   ui/scripts/win-titlebar.ps1 -App <GitVolt.exe> -Out <dir>
#
# The steps: the frame (the system's border at the sides and bottom, none at the top); a drag of
# the title bar; a double click maximizes to the work area and restores; with the window clear of
# the taskbar, each border and the bottom-right corner resize it (cursors too), and the page's top
# edge; the edges' pixels, active and not; the buttons; the window menu (right click, Alt+Space);
# Aero Snap; shake minimizes another window and brings it back (after Notepad's own title bar, the
# control: some sessions don't shake); a restart keeps the size; "Use the system title bar"; and last, best
# effort, the same at 150% display scaling, switched live (DisplayConfigSetDeviceInfo, as Settings
# does; no sign-out).
param(
  [Parameter(Mandatory)] [string]$App,
  [Parameter(Mandatory)] [string]$Out
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $Out | Out-Null
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Collections.Generic;
public static class W {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; public override string ToString() { return string.Format("({0},{1})-({2},{3}) {4}x{5}", L, T, R, B, R - L, B - T); } }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [StructLayout(LayoutKind.Sequential)] public struct MONITORINFO { public int cbSize; public RECT rcMonitor; public RECT rcWork; public uint dwFlags; }
  [StructLayout(LayoutKind.Sequential)] public struct CURSORINFO { public int cbSize; public int flags; public IntPtr hCursor; public POINT pt; }
  [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public MOUSEINPUT mi; }
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)] public struct DEVMODE {
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string dmDeviceName;
    public short dmSpecVersion, dmDriverVersion, dmSize, dmDriverExtra; public int dmFields;
    public int dmPositionX, dmPositionY, dmDisplayOrientation, dmDisplayFixedOutput;
    public short dmColor, dmDuplex, dmYResolution, dmTTOption, dmCollate;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string dmFormName;
    public short dmLogPixels; public int dmBitsPerPel, dmPelsWidth, dmPelsHeight, dmDisplayFlags, dmDisplayFrequency;
    public int dmICMMethod, dmICMIntent, dmMediaType, dmDitherType, dmReserved1, dmReserved2, dmPanningWidth, dmPanningHeight;
  }
  // DisplayConfigGetDeviceInfo / SetDeviceInfo's undocumented DPI scale packets (types -3 and -4),
  // what Settings › Display › Scale uses: the scale relative to the recommended one.
  [StructLayout(LayoutKind.Sequential)] public struct DCHEADER { public int type; public int size; public uint adapterLow; public int adapterHigh; public uint id; }
  [StructLayout(LayoutKind.Sequential)] public struct DPIGET { public DCHEADER h; public int minRel, curRel, maxRel; }
  [StructLayout(LayoutKind.Sequential)] public struct DPISET { public DCHEADER h; public int rel; }
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern bool GetCursorInfo(ref CURSORINFO c);
  [DllImport("user32.dll")] public static extern IntPtr LoadCursor(IntPtr i, int id);
  [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] i, int size);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, IntPtr extra);
  [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint code, uint type);
  [DllImport("user32.dll")] public static extern IntPtr MonitorFromWindow(IntPtr h, uint f);
  [DllImport("user32.dll")] public static extern bool GetMonitorInfo(IntPtr m, ref MONITORINFO i);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int i);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int ChangeDisplaySettings(ref DEVMODE d, int f);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern bool EnumDisplaySettings(string d, int m, ref DEVMODE dm);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int size);
  [DllImport("user32.dll")] static extern int GetDisplayConfigBufferSizes(uint flags, out uint paths, out uint modes);
  [DllImport("user32.dll")] static extern int QueryDisplayConfig(uint flags, ref uint paths, byte[] pathInfo, ref uint modes, byte[] modeInfo, IntPtr topology);
  [DllImport("user32.dll")] static extern int DisplayConfigGetDeviceInfo(ref DPIGET p);
  [DllImport("user32.dll")] static extern int DisplayConfigSetDeviceInfo(ref DPISET p);

  public static List<IntPtr> TopLevel(uint pid, string cls) {
    var found = new List<IntPtr>();
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if ((pid == 0 || p == pid) && IsWindowVisible(h)) {
        if (string.IsNullOrEmpty(cls) || Class(h) == cls) found.Add(h);
      }
      return true;
    }, IntPtr.Zero);
    return found;
  }
  public static RECT Mk(int l, int t, int r, int b) { var x = new RECT(); x.L = l; x.T = t; x.R = r; x.B = b; return x; }
  public static string Class(IntPtr h) { var s = new StringBuilder(256); GetClassName(h, s, 256); return s.ToString(); }
  public static string Title(IntPtr h) { var s = new StringBuilder(512); GetWindowText(h, s, 512); return s.ToString(); }
  public static RECT Rect(IntPtr h) { RECT r; GetWindowRect(h, out r); return r; }
  public static RECT Frame(IntPtr h) { RECT r; DwmGetWindowAttribute(h, 9, out r, Marshal.SizeOf(typeof(RECT))); return r; }
  public static RECT Client(IntPtr h) { RECT r; GetClientRect(h, out r); var p = new POINT(); ClientToScreen(h, ref p); r.L += p.X; r.R += p.X; r.T += p.Y; r.B += p.Y; return r; }
  public static RECT Work(IntPtr h) { var i = new MONITORINFO(); i.cbSize = Marshal.SizeOf(i); GetMonitorInfo(MonitorFromWindow(h, 2), ref i); return i.rcWork; }
  public static RECT Monitor(IntPtr h) { var i = new MONITORINFO(); i.cbSize = Marshal.SizeOf(i); GetMonitorInfo(MonitorFromWindow(h, 2), ref i); return i.rcMonitor; }
  public static IntPtr Cursor() { var c = new CURSORINFO(); c.cbSize = Marshal.SizeOf(c); GetCursorInfo(ref c); return c.hCursor; }
  public static void Place(IntPtr h, int x, int y) { SetWindowPos(h, IntPtr.Zero, x, y, 0, 0, 0x0001 | 0x0004 | 0x0010); } // NOSIZE | NOZORDER | NOACTIVATE
  static void Send(uint flags, int x, int y) {
    int vx = GetSystemMetrics(76), vy = GetSystemMetrics(77), vw = GetSystemMetrics(78), vh = GetSystemMetrics(79);
    var i = new INPUT[1];
    i[0].type = 0;
    i[0].mi.dx = (int)Math.Round((x - vx) * 65535.0 / (vw - 1));
    i[0].mi.dy = (int)Math.Round((y - vy) * 65535.0 / (vh - 1));
    i[0].mi.dwFlags = flags | 0x8000 | 0x4000 | 0x0001; // ABSOLUTE | VIRTUALDESK | MOVE
    SendInput(1, i, Marshal.SizeOf(typeof(INPUT)));
  }
  public static void Move(int x, int y) { Send(0, x, y); }
  public static void Down(int x, int y) { Send(0x0002, x, y); }
  public static void Up(int x, int y) { Send(0x0004, x, y); }
  public static void RDown(int x, int y) { Send(0x0008, x, y); }
  public static void RUp(int x, int y) { Send(0x0010, x, y); }
  public static int Resolution(int w, int h) {
    var d = new DEVMODE(); d.dmSize = (short)Marshal.SizeOf(d);
    EnumDisplaySettings(null, -1, ref d);
    d.dmPelsWidth = w; d.dmPelsHeight = h; d.dmFields = 0x80000 | 0x100000;
    return ChangeDisplaySettings(ref d, 0);
  }
  static readonly int[] Scales = { 100, 125, 150, 175, 200, 225, 250, 300, 350, 400, 450, 500 };
  /// The first display's scale set to `percent`, live; says what it found and did.
  public static string SetScale(int percent) {
    uint np, nm;
    int e = GetDisplayConfigBufferSizes(2, out np, out nm); // QDC_ONLY_ACTIVE_PATHS
    if (e != 0) return "GetDisplayConfigBufferSizes failed: " + e;
    var paths = new byte[np * 72]; var modes = new byte[nm * 64];
    e = QueryDisplayConfig(2, ref np, paths, ref nm, modes, IntPtr.Zero);
    if (e != 0 || np == 0) return "QueryDisplayConfig failed: " + e + ", " + np + " paths";
    var g = new DPIGET();
    g.h.type = -3; g.h.size = Marshal.SizeOf(g);
    g.h.adapterLow = BitConverter.ToUInt32(paths, 0); g.h.adapterHigh = BitConverter.ToInt32(paths, 4); g.h.id = BitConverter.ToUInt32(paths, 8);
    e = DisplayConfigGetDeviceInfo(ref g);
    if (e != 0) return "reading the scale failed: " + e;
    int recommended = Math.Abs(g.minRel), target = Array.IndexOf(Scales, percent);
    string found = string.Format("scales offered {0}% to {1}%, now {2}% (recommended {3}%)", Scales[recommended + g.minRel], Scales[Math.Min(Scales.Length - 1, recommended + g.maxRel)], Scales[recommended + g.curRel], Scales[recommended]);
    if (target < 0 || target - recommended > g.maxRel || target - recommended < g.minRel) return found + "; " + percent + "% isn't offered";
    var s = new DPISET();
    s.h = g.h; s.h.type = -4; s.h.size = Marshal.SizeOf(s); s.rel = target - recommended;
    e = DisplayConfigSetDeviceInfo(ref s);
    return found + "; set " + percent + "%: " + (e == 0 ? "ok" : "error " + e);
  }
}
'@

# Physical pixels whatever the scale, as GitVolt (per-monitor aware) has them.
$aware = [W]::SetProcessDpiAwarenessContext([IntPtr]::new(-4))

$results = Join-Path $Out 'results.md'
"# GitVolt title bar on $((Get-CimInstance Win32_OperatingSystem).Caption) $([Environment]::OSVersion.Version)`n" | Set-Content $results
$script:failures = 0
function Log([string]$line) { Write-Host $line; Add-Content $results $line }
function Check([string]$name, [bool]$ok, [string]$detail) {
  $mark = if ($ok) { 'PASS' } else { $script:failures++; 'FAIL' }
  Log "- **$mark** $name — $detail"
}
function Shot([string]$name, $rect = $null) {
  $vx = [W]::GetSystemMetrics(76); $vy = [W]::GetSystemMetrics(77); $vw = [W]::GetSystemMetrics(78); $vh = [W]::GetSystemMetrics(79)
  if ($rect) { $x = $rect.L; $y = $rect.T; $w = $rect.R - $rect.L; $h = $rect.B - $rect.T } else { $x = $vx; $y = $vy; $w = $vw; $h = $vh }
  $bmp = New-Object System.Drawing.Bitmap $w, $h
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($x, $y, 0, 0, $bmp.Size)
  $bmp.Save((Join-Path $Out "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
}
function Pixel([int]$x, [int]$y) {
  $bmp = New-Object System.Drawing.Bitmap 1, 1
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($x, $y, 0, 0, $bmp.Size)
  $c = $bmp.GetPixel(0, 0); $g.Dispose(); $bmp.Dispose(); $c
}
function Rgb($c) { "$($c.R),$($c.G),$($c.B)" }
function Wait([int]$ms) { Start-Sleep -Milliseconds $ms }
function Drag([int]$x0, [int]$y0, [int]$x1, [int]$y1, [int]$steps = 20) {
  [W]::Move($x0, $y0); Wait 300
  [W]::Down($x0, $y0); Wait 200
  for ($i = 1; $i -le $steps; $i++) {
    $x = [int]($x0 + ($x1 - $x0) * $i / $steps); $y = [int]($y0 + ($y1 - $y0) * $i / $steps)
    [W]::Move($x, $y); Wait 40
  }
  Wait 400
  [W]::Up($x1, $y1); Wait 1200
}
function Click([int]$x, [int]$y) { [W]::Move($x, $y); Wait 250; [W]::Down($x, $y); Wait 60; [W]::Up($x, $y); Wait 900 }
function DoubleClick([int]$x, [int]$y) { [W]::Move($x, $y); Wait 250; [W]::Down($x, $y); Wait 40; [W]::Up($x, $y); Wait 60; [W]::Down($x, $y); Wait 40; [W]::Up($x, $y); Wait 1500 }
function RightClick([int]$x, [int]$y) { [W]::Move($x, $y); Wait 250; [W]::RDown($x, $y); Wait 60; [W]::RUp($x, $y); Wait 1200 }
# Keys with their scan codes, as a keyboard sends them (Chromium names `code` from the scan code).
function KeyDown([byte]$vk) { [W]::keybd_event($vk, [byte][W]::MapVirtualKey($vk, 0), 0, [IntPtr]::Zero) }
function KeyUp([byte]$vk) { [W]::keybd_event($vk, [byte][W]::MapVirtualKey($vk, 0), 2, [IntPtr]::Zero) }
function Key([byte]$vk) { KeyDown $vk; Wait 50; KeyUp $vk; Wait 600 }
function Menus { @([W]::TopLevel(0, '#32768')) }
function Fmt($r) { $r.ToString() }
function Mid($r) { [int](($r.L + $r.R) / 2) }
function Same($a, $b) { $a.L -eq $b.L -and $a.T -eq $b.T -and $a.R -eq $b.R -and $a.B -eq $b.B }
# The title bar and a little around it; `s` scales the crop's height with the display.
function BarShot([string]$name, $c, [double]$s = 1) { Shot $name ([W]::Mk($c.L - 16, $c.T - 8, $c.R + 16, $c.T + [int](72 * $s))) }

# A failing statement is logged and the script goes on with the next one, so one broken check
# doesn't hide the others; without a window there's nothing left to check.
trap {
  $script:failures++
  Log "- **FAIL** error at line $($_.InvocationInfo.ScriptLineNumber): $($_.Exception.Message)"
  if ($_.Exception.Message -eq 'no GitVolt window') { break }
  continue
}

$set = [W]::Resolution(1920, 1080)
Log "Resolution 1920x1080: ChangeDisplaySettings=$set; screen now $([W]::GetSystemMetrics(0))x$([W]::GetSystemMetrics(1)); per-monitor DPI aware: $aware"

# Shake is off by default on Windows 11 (Settings › System › Multitasking › Title bar window
# shake); Explorer reads the switch when it starts.
$advanced = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced'
$was = (Get-ItemProperty $advanced -Name DisallowShaking -ErrorAction SilentlyContinue).DisallowShaking
New-ItemProperty -Path $advanced -Name DisallowShaking -Value 0 -PropertyType DWord -Force | Out-Null
Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue
Wait 6000
if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) { Start-Process explorer; Wait 6000 }
Log "Shake turned on (DisallowShaking was '$was'), Explorer restarted: $(@(Get-Process explorer -ErrorAction SilentlyContinue).Count) process(es)"

function Launch {
  $p = Start-Process -FilePath $App -PassThru
  $hwnd = [IntPtr]::Zero
  for ($i = 0; $i -lt 90 -and $hwnd -eq [IntPtr]::Zero; $i++) {
    Wait 1000
    foreach ($id in @(Get-Process -Name GitVolt -ErrorAction SilentlyContinue | ForEach-Object Id)) {
      foreach ($w in [W]::TopLevel([uint32]$id, '')) { if ([W]::Title($w) -like '*GitVolt*') { $hwnd = $w } }
    }
  }
  if ($hwnd -eq [IntPtr]::Zero) {
    Log (Get-Process | Where-Object { $_.ProcessName -like '*GitVolt*' } | Format-Table Id, ProcessName, MainWindowTitle | Out-String)
    Shot 'no-window'
    throw 'no GitVolt window'
  }
  Wait 10000 # the page loads and renders
  [W]::SetForegroundWindow($hwnd) | Out-Null; Wait 500
  @{ Process = $p; Hwnd = $hwnd }
}
function StopAll { Get-Process | Where-Object { $_.ProcessName -like '*GitVolt*' } | Stop-Process -Force -ErrorAction SilentlyContinue; Wait 2000 }
$settings = Join-Path $env:APPDATA 'gitvolt\settings.json'
function SystemTitleBar([bool]$on) {
  $s = Get-Content -Raw $settings | ConvertFrom-Json
  $s | Add-Member -Force -NotePropertyName systemTitleBar -NotePropertyValue $on
  $s | ConvertTo-Json -Depth 64 | Set-Content -Encoding utf8 $settings
}

$run = Launch
$h = $run.Hwnd
$r0 = [W]::Rect($h); $f0 = [W]::Frame($h); $c0 = [W]::Client($h); $work = [W]::Work($h)
Log "`nWindow $h class $([W]::Class($h)) title '$([W]::Title($h))' dpi $([W]::GetDpiForWindow($h)) style 0x$('{0:X8}' -f [W]::GetWindowLong($h, -16))"
Log "rect $(Fmt $r0); visible frame (DWM) $(Fmt $f0); client $(Fmt $c0); work area $(Fmt $work)`n"
Shot '01-initial'
BarShot '01-titlebar' $c0
Check 'the system border at the sides and bottom, none at the top' (($c0.L - $r0.L) -gt 0 -and ($r0.R - $c0.R) -gt 0 -and ($r0.B - $c0.B) -gt 0 -and $c0.T -eq $r0.T) "border L=$($c0.L - $r0.L) R=$($r0.R - $c0.R) B=$($r0.B - $c0.B) T=$($c0.T - $r0.T)"
Check 'the visible frame hugs the client area' ([Math]::Abs($f0.L - $c0.L) -le 1 -and [Math]::Abs($f0.R - $c0.R) -le 1 -and [Math]::Abs($f0.B - $c0.B) -le 1 -and $f0.T -eq $c0.T) "frame $(Fmt $f0) client $(Fmt $c0)"

# --- the edges: what's drawn at the window's top row, just above it, and on the sides ---
Log "`n## Edges"
function Edges([string]$when, $c) {
  $mx = Mid $c; $my = [int](($c.T + $c.B) / 2)
  Log "$($when): top row $(Rgb (Pixel $mx $c.T)), the row above $(Rgb (Pixel $mx ($c.T - 1))), bar $(Rgb (Pixel $mx ($c.T + 4))); left edge $(Rgb (Pixel ($c.L - 1) $my)) / $(Rgb (Pixel $c.L $my)); bottom edge $(Rgb (Pixel $mx $c.B)) / $(Rgb (Pixel $mx ($c.B - 1)))"
  Shot "01-top-left-$when" ([W]::Mk($c.L - 12, $c.T - 12, $c.L + 160, $c.T + 40))
  Shot "01-top-right-$when" ([W]::Mk($c.R - 200, $c.T - 12, $c.R + 12, $c.T + 40))
}
# Clear of the desktop's own edges first, so the pixels around the window are the desktop's.
[W]::Place($h, 150, 60); Wait 800
$c = [W]::Client($h)
Edges 'active' $c
$pad = Start-Process notepad -PassThru
Wait 2500
$notepad = [IntPtr]::Zero
foreach ($w in [W]::TopLevel([uint32]$pad.Id, '')) { $notepad = $w }
if ($notepad -eq [IntPtr]::Zero) { foreach ($w in [W]::TopLevel(0, 'Notepad')) { $notepad = $w } }
if ($notepad -ne [IntPtr]::Zero) {
  # Notepad in front, away from GitVolt's top edge: GitVolt is inactive.
  [W]::SetWindowPos($notepad, [IntPtr]::Zero, 1100, 500, 600, 400, 0x0004) | Out-Null
  [W]::SetForegroundWindow($notepad) | Out-Null; Wait 1000
  Edges 'inactive' $c
  BarShot '01-titlebar-inactive' $c
}
Log "Notepad: window $notepad ($(if ($notepad -ne [IntPtr]::Zero) { [W]::Class($notepad) }))"
[W]::SetForegroundWindow($h) | Out-Null; Wait 800

# --- drag ---
Log "`n## Drag"
$r0 = [W]::Rect($h); $c0 = [W]::Client($h)
$tx = Mid $c0; $ty = $c0.T + 15
Drag $tx $ty ($tx - 200) ($ty + 100)
$r1 = [W]::Rect($h)
$dx = $r1.L - $r0.L; $dy = $r1.T - $r0.T
Shot '02-dragged'
Check 'dragging the title moves the window with the pointer' ($dx -le -150 -and $dx -ge -200 -and $dy -ge 50 -and $dy -le 100) "pointer (-200,+100), window ($dx,$dy): $(Fmt $r0) -> $(Fmt $r1)"
Check 'a drag keeps the size' (($r1.R - $r1.L) -eq ($r0.R - $r0.L) -and ($r1.B - $r1.T) -eq ($r0.B - $r0.T)) (Fmt $r1)

# --- double click ---
Log "`n## Double click"
$r1 = [W]::Rect($h); $c = [W]::Client($h)
DoubleClick (Mid $c) ($c.T + 15)
$zoomed = [W]::IsZoomed($h); $cz = [W]::Client($h); $wz = [W]::Work($h)
Shot '03-maximized'
BarShot '03-maximized-titlebar' $cz
Check 'a double click on the title bar maximizes' $zoomed "IsZoomed=$zoomed rect $(Fmt ([W]::Rect($h)))"
Check 'maximized, the client area is the work area: the taskbar stays visible' (Same $cz $wz) "client $(Fmt $cz) work $(Fmt $wz)"
Log "maximized: top row $(Rgb (Pixel (Mid $cz) $cz.T)), bar $(Rgb (Pixel (Mid $cz) ($cz.T + 4)))"
DoubleClick (Mid $wz) ($wz.T + 15)
$r2 = [W]::Rect($h)
Check 'a double click again restores' ((-not [W]::IsZoomed($h)) -and (Same $r2 $r1)) "rect $(Fmt $r2) (was $(Fmt $r1))"

# --- resize: up and away from the taskbar first, so the bottom border is on the screen ---
Log "`n## Resize"
[W]::Place($h, 150, 20); Wait 800
$c = [W]::Client($h); $r = [W]::Rect($h)
Log "placed at $(Fmt $r); client $(Fmt $c); work area bottom $([W]::Work($h).B)"
$midY = [int](($c.T + $c.B) / 2); $midX = Mid $c
[W]::Move($c.R + 3, $midY); Wait 400
$cur = [W]::Cursor(); $we = [W]::LoadCursor([IntPtr]::Zero, 32644)
Check 'the right border shows the left-right resize cursor' ($cur -eq $we) "cursor $cur, IDC_SIZEWE $we"
Drag ($c.R + 3) $midY ($c.R + 123) $midY
$ra = [W]::Rect($h)
Check 'the right border resizes' (($ra.R - $r.R) -eq 120 -and $ra.L -eq $r.L) "R $($r.R) -> $($ra.R) (+120 asked)"
$c = [W]::Client($h); $r = $ra
[W]::Move($midX, $c.B + 3); Wait 400
$cur = [W]::Cursor(); $ns = [W]::LoadCursor([IntPtr]::Zero, 32645)
Check 'the bottom border shows the up-down resize cursor' ($cur -eq $ns) "cursor $cur, IDC_SIZENS $ns"
Drag $midX ($c.B + 3) $midX ($c.B + 83)
$rb = [W]::Rect($h)
Check 'the bottom border resizes' (($rb.B - $r.B) -eq 80 -and $rb.T -eq $r.T) "B $($r.B) -> $($rb.B) (+80 asked)"
$c = [W]::Client($h); $r = $rb
[W]::Move($c.R + 3, $c.B + 3); Wait 400
$cur = [W]::Cursor(); $nwse = [W]::LoadCursor([IntPtr]::Zero, 32642)
Check 'the bottom-right corner shows the diagonal resize cursor' ($cur -eq $nwse) "cursor $cur, IDC_SIZENWSE $nwse"
Drag ($c.R + 3) ($c.B + 3) ($c.R - 97) ($c.B - 57)
$rc = [W]::Rect($h)
Check 'the bottom-right corner resizes' (($rc.R - $r.R) -eq -100 -and ($rc.B - $r.B) -eq -60 -and $rc.L -eq $r.L -and $rc.T -eq $r.T) "R $($r.R) -> $($rc.R), B $($r.B) -> $($rc.B) (-100,-60 asked)"
$c = [W]::Client($h); $r = $rc
Drag ($c.L - 3) $midY ($c.L - 93) $midY
$rd = [W]::Rect($h)
Check 'the left border resizes' (($rd.L - $r.L) -eq -90 -and $rd.R -eq $r.R) "L $($r.L) -> $($rd.L) (-90 asked)"
$c = [W]::Client($h); $r = $rd
[W]::Place($h, $r.L, 120); Wait 800
$c = [W]::Client($h); $r = [W]::Rect($h); $midX = Mid $c
[W]::Move($midX, $c.T + 1); Wait 400
$cur = [W]::Cursor()
Check "the top edge (the page's) shows the up-down resize cursor" ($cur -eq $ns) "cursor $cur, IDC_SIZENS $ns"
Drag $midX ($c.T + 1) $midX ($c.T - 69)
$re = [W]::Rect($h)
Check 'the top edge resizes' ([Math]::Abs(($re.T - $r.T) + 70) -le 4 -and $re.B -eq $r.B) "T $($r.T) -> $($re.T) (-70 asked)"
$c = [W]::Client($h); $r = $re
Drag ($c.L - 3) ($c.T + 2) ($c.L - 43) ($c.T - 28)
$rf = [W]::Rect($h)
Check 'the top-left corner resizes from the left border' (($rf.L - $r.L) -eq -40 -and ($rf.T - $r.T) -eq -30) "L $($r.L) -> $($rf.L), T $($r.T) -> $($rf.T) (-40,-30 asked)"
Shot '04-resized'

# --- buttons ---
Log "`n## Buttons"
$c = [W]::Client($h); $by = $c.T + 15
[W]::Move($c.R - 23, $by); Wait 700
$px = Pixel ($c.R - 40) ($c.T + 25)
Shot '05-close-hover' ([W]::Mk($c.R - 300, $c.T, $c.R + 8, $c.T + 64))
Check 'close turns red on hover' ([Math]::Abs($px.R - 0xC4) -le 12 -and [Math]::Abs($px.G - 0x2B) -le 12 -and [Math]::Abs($px.B - 0x1C) -le 12) "pixel $(Rgb $px) (want 196,43,28)"
[W]::Move($c.R - 69, $by); Wait 700
Shot '05-maximize-hover' ([W]::Mk($c.R - 300, $c.T, $c.R + 8, $c.T + 64))
Click ($c.R - 69) $by
$wz = [W]::Work($h)
Check 'the maximize button maximizes' ([W]::IsZoomed($h)) "rect $(Fmt ([W]::Rect($h)))"
Shot '05-restore-button' ([W]::Mk($wz.R - 300, $wz.T, $wz.R, $wz.T + 64))
Click ($wz.R - 69) ($wz.T + 15)
Check 'the restore button restores' (-not [W]::IsZoomed($h)) "rect $(Fmt ([W]::Rect($h)))"
$c = [W]::Client($h)
Click ($c.R - 115) ($c.T + 15)
Check 'the minimize button minimizes' ([W]::IsIconic($h)) "IsIconic=$([W]::IsIconic($h))"
[W]::ShowWindow($h, 9) | Out-Null; Wait 800; [W]::SetForegroundWindow($h) | Out-Null; Wait 800

# --- window menu ---
Log "`n## Window menu"
$c = [W]::Client($h); $tx = (Mid $c) + 100; $ty = $c.T + 15
RightClick $tx $ty
$menus = Menus
Shot '06-right-click-menu'
Check 'a right click on the title bar shows the window menu at the pointer' ($menus.Count -gt 0) "menus: $(($menus | ForEach-Object { Fmt ([W]::Rect($_)) }) -join ', '); pointer at $tx,$ty"
Key 0x1B
Check 'Esc closes it' ((Menus).Count -eq 0) "menus left: $((Menus).Count)"
RightClick $tx $ty
Key 0x58 # Ma&ximize
Check "the menu's Maximize maximizes" ([W]::IsZoomed($h)) "IsZoomed=$([W]::IsZoomed($h))"
$wz = [W]::Work($h)
RightClick ((Mid $wz) + 100) ($wz.T + 15)
Shot '06-menu-when-maximized'
Key 0x52 # &Restore
Check "the menu's Restore restores" (-not [W]::IsZoomed($h)) "IsZoomed=$([W]::IsZoomed($h))"

Log "`n## Alt+Space"
$c = [W]::Client($h)
Click (Mid $c) ($c.T + 15)
KeyDown 0x12; Wait 60
KeyDown 0x20; Wait 60
KeyUp 0x20; Wait 60
KeyUp 0x12; Wait 1200
$menus = Menus
Shot '07-alt-space'
Check 'Alt+Space shows the window menu' ($menus.Count -gt 0) "menus: $(($menus | ForEach-Object { Fmt ([W]::Rect($_)) }) -join ', '); client $(Fmt ([W]::Client($h)))"
Key 0x1B; Key 0x1B

# --- snap ---
Log "`n## Aero Snap"
$c = [W]::Client($h); $mon = [W]::Monitor($h); $work = [W]::Work($h)
Drag (Mid $c) ($c.T + 15) (Mid $c) ($mon.T) 40
Wait 800
Shot '08-snap-top'
Check 'dragging to the top of the screen maximizes' ([W]::IsZoomed($h)) "rect $(Fmt ([W]::Rect($h)))"
if ([W]::IsZoomed($h)) { DoubleClick (Mid $work) ($work.T + 15) }
$c = [W]::Client($h)
Drag (Mid $c) ($c.T + 15) ($mon.L) ([int](($mon.T + $mon.B) / 2)) 40
Wait 1000
$cs = [W]::Client($h)
Shot '08-snap-left'
$half = [int](($work.R - $work.L) / 2)
Check 'dragging to the left of the screen snaps to the left half' ([Math]::Abs($cs.L - $work.L) -le 1 -and [Math]::Abs(($cs.R - $cs.L) - $half) -le 2 -and [Math]::Abs($cs.T - $work.T) -le 1 -and [Math]::Abs($cs.B - $work.B) -le 1) "client $(Fmt $cs), work $(Fmt $work)"
Key 0x1B # snap assist, if any

# --- shake: a title bar shaken minimizes the other windows, and shaken again brings them back.
# Notepad's own title bar first, as the control: whether this session shakes at all, and how fast. ---
Log "`n## Shake"
$c = [W]::Client($h)
Drag (Mid $c) ($c.T + 15) 700 200
# Press at x0,y0, past the page's 4 px threshold, a moment for Windows' move loop to start, then
# five swings of 120 px each way, `half` ms a swing.
function Shake([int]$x0, [int]$y0, [int]$half) {
  [W]::Move($x0, $y0); Wait 300
  [W]::Down($x0, $y0); Wait 150
  for ($i = 1; $i -le 4; $i++) { [W]::Move($x0 + 2 * $i, $y0); Wait 30 }
  Wait 500
  for ($n = 0; $n -lt 5; $n++) {
    foreach ($to in @(120, -120)) {
      for ($i = -3; $i -le 4; $i++) { [W]::Move($x0 + [int]($to * $i / 4), $y0); Wait ([int]($half / 8)) }
    }
  }
  [W]::Move($x0, $y0); Wait 300
  [W]::Up($x0, $y0); Wait 2000
}
if ($notepad -ne [IntPtr]::Zero) {
  [W]::ShowWindow($notepad, 9) | Out-Null
  [W]::SetWindowPos($notepad, [IntPtr]::Zero, 80, 600, 600, 380, (0x0010 -bor 0x0004)) | Out-Null
  $speed = 0
  foreach ($half in @(80, 160, 40)) {
    [W]::SetForegroundWindow($notepad) | Out-Null; Wait 800
    $nr = [W]::Rect($notepad)
    Shake (Mid $nr) ($nr.T + 12) $half
    $control = [W]::IsIconic($h)
    Log "Notepad shaken by its own title bar, $half ms a swing: GitVolt minimized $control"
    if ($control) {
      Shake (Mid ([W]::Rect($notepad))) ([W]::Rect($notepad).T + 12) $half
      Log "and again: GitVolt minimized $([W]::IsIconic($h))"
      $speed = $half
      break
    }
  }
  [W]::ShowWindow($h, 9) | Out-Null; [W]::ShowWindow($notepad, 9) | Out-Null
  if ($speed) {
    [W]::SetForegroundWindow($h) | Out-Null; Wait 800
    $before = [W]::IsIconic($notepad)
    $c = [W]::Client($h)
    Shake (Mid $c) ($c.T + 15) $speed
    $after = [W]::IsIconic($notepad)
    Shot '09-shaken'
    Check 'shaking the title bar minimizes the other window' ((-not $before) -and $after -and -not [W]::IsIconic($h)) "Notepad minimized before $before, after $after; GitVolt minimized $([W]::IsIconic($h))"
    $c = [W]::Client($h)
    Shake (Mid $c) ($c.T + 15) $speed
    $again = [W]::IsIconic($notepad)
    Shot '09-shaken-again'
    Check 'shaking it again brings the other window back' ($after -and -not $again) "Notepad minimized $again"
  } else {
    Log "Shaking Notepad's own title bar minimized nothing: this session doesn't shake; GitVolt's wasn't checked."
  }
} else {
  Check 'Notepad opened, for the shake' $false 'no Notepad window'
}
Stop-Process -Id $pad.Id -Force -ErrorAction SilentlyContinue
[W]::ShowWindow($h, 9) | Out-Null; [W]::SetForegroundWindow($h) | Out-Null; Wait 800

# --- restart: the size comes back without drifting ---
Log "`n## Restart"
[W]::SetForegroundWindow($h) | Out-Null; Wait 500
$c = [W]::Client($h)
Drag (Mid $c) ($c.T + 15) 900 300
$cBefore = [W]::Client($h); $rBefore = [W]::Rect($h)
Click ($cBefore.R - 23) ($cBefore.T + 15)
$exited = $run.Process.WaitForExit(20000)
Check 'the close button closes GitVolt' $exited "exited=$exited"
if (-not $exited) { Stop-Process -Id $run.Process.Id -Force }
Wait 3000
$run = Launch; $h = $run.Hwnd
$cAfter = [W]::Client($h); $rAfter = [W]::Rect($h)
Shot '10-restarted'
Check 'after a restart the client area has the same size and place' (Same $cAfter $cBefore) "client $(Fmt $cBefore) -> $(Fmt $cAfter); window $(Fmt $rBefore) -> $(Fmt $rAfter)"
Stop-Process -Id $run.Process.Id -Force; Wait 3000
StopAll

# --- the system title bar setting ---
Log "`n## Use the system title bar"
if (Test-Path $settings) {
  SystemTitleBar $true
  $run = Launch; $h = $run.Hwnd
  $r = [W]::Rect($h); $c = [W]::Client($h)
  Shot '11-system-title-bar'
  Check 'with the setting on, the system caption is back' (($c.T - $r.T) -ge 20) "client top $($c.T - $r.T) px below the window's; rect $(Fmt $r) client $(Fmt $c)"
  Stop-Process -Id $run.Process.Id -Force
  StopAll
  SystemTitleBar $false
} else {
  Check 'settings.json exists' $false "$settings missing"
}

# --- 150% display scaling, best effort: switched live, as Settings does ---
Log "`n## 150% scaling"
$run = Launch; $h = $run.Hwnd
$cLive = [W]::Client($h)
$scale = [W]::SetScale(150)
Log "DisplayConfigSetDeviceInfo: $scale"
Wait 4000
$dpi = [W]::GetDpiForWindow($h)
Log "GitVolt's window at $dpi dpi after the switch: rect $(Fmt ([W]::Rect($h))), client $(Fmt ([W]::Client($h))) (was $(Fmt $cLive)); screen $([W]::GetSystemMetrics(0))x$([W]::GetSystemMetrics(1))"
Shot '12-scaled-live'
if ($dpi -eq 144) {
  $s = 1.5
  $c = [W]::Client($h); $r = [W]::Rect($h)
  Check 'live at 150%: the border is the system frame at 144 dpi' (($c.L - $r.L) -ge 10 -and ($r.B - $c.B) -ge 10 -and $c.T -eq $r.T) "border L=$($c.L - $r.L) R=$($r.R - $c.R) B=$($r.B - $c.B) T=$($c.T - $r.T)"
  Click ($c.R - [int](23 * $s)) ($c.T + [int](15 * $s))
  $exited = $run.Process.WaitForExit(20000)
  Check 'at 150% the close button closes GitVolt' $exited "exited=$exited"
  StopAll
  $run = Launch; $h = $run.Hwnd
  $r0 = [W]::Rect($h); $c0 = [W]::Client($h); $f0 = [W]::Frame($h)
  Log "Started at 150%: dpi $([W]::GetDpiForWindow($h)); rect $(Fmt $r0); frame $(Fmt $f0); client $(Fmt $c0)"
  Shot '12-scaled'
  BarShot '12-scaled-titlebar' $c0 $s
  Check 'at 150%, the border at the sides and bottom, none at the top' (($c0.L - $r0.L) -ge 10 -and ($r0.R - $c0.R) -ge 10 -and ($r0.B - $c0.B) -ge 10 -and $c0.T -eq $r0.T) "border L=$($c0.L - $r0.L) R=$($r0.R - $c0.R) B=$($r0.B - $c0.B) T=$($c0.T - $r0.T)"
  Check 'at 150%, the visible frame hugs the client area' ([Math]::Abs($f0.L - $c0.L) -le 2 -and [Math]::Abs($f0.R - $c0.R) -le 2 -and [Math]::Abs($f0.B - $c0.B) -le 2 -and $f0.T -eq $c0.T) "frame $(Fmt $f0) client $(Fmt $c0)"
  # Smaller than the screen, so its borders are on it.
  [W]::SetWindowPos($h, [IntPtr]::Zero, 100, 20, 1500, 850, (0x0004 -bor 0x0010)) | Out-Null; Wait 800
  $c = [W]::Client($h); $r = [W]::Rect($h); $midY = [int](($c.T + $c.B) / 2)
  [W]::Move($c.R + 4, $midY); Wait 400
  Check 'at 150%, the right border shows the resize cursor' ([W]::Cursor() -eq [W]::LoadCursor([IntPtr]::Zero, 32644)) "cursor $([W]::Cursor())"
  Drag ($c.R + 4) $midY ($c.R + 124) $midY
  $ra = [W]::Rect($h)
  Check 'at 150%, the right border resizes' (($ra.R - $r.R) -eq 120) "R $($r.R) -> $($ra.R) (+120 asked)"
  $c = [W]::Client($h); $r = [W]::Rect($h)
  Drag (Mid $c) ($c.B + 4) (Mid $c) ($c.B + 64)
  $rb = [W]::Rect($h)
  Check 'at 150%, the bottom border resizes' (($rb.B - $r.B) -eq 60) "B $($r.B) -> $($rb.B) (+60 asked)"
  $c = [W]::Client($h); $r = [W]::Rect($h)
  [W]::Move((Mid $c), $c.T + 1); Wait 400
  Check "at 150%, the page's top edge shows the resize cursor" ([W]::Cursor() -eq [W]::LoadCursor([IntPtr]::Zero, 32645)) "cursor $([W]::Cursor())"
  $c = [W]::Client($h)
  DoubleClick (Mid $c) ($c.T + [int](15 * $s))
  $cz = [W]::Client($h); $wz = [W]::Work($h)
  Shot '12-scaled-maximized'
  Check 'at 150%, a double click maximizes to the work area' ([W]::IsZoomed($h) -and (Same $cz $wz)) "client $(Fmt $cz) work $(Fmt $wz)"
  DoubleClick (Mid $wz) ($wz.T + [int](15 * $s))
  $cBefore = [W]::Client($h)
  Check 'at 150%, a double click again restores' (-not [W]::IsZoomed($h)) "client $(Fmt $cBefore)"
  Click ($cBefore.R - [int](23 * $s)) ($cBefore.T + [int](15 * $s))
  $exited = $run.Process.WaitForExit(20000)
  if (-not $exited) { StopAll }
  Wait 3000
  $run = Launch; $h = $run.Hwnd
  $cAfter = [W]::Client($h)
  Shot '12-scaled-restarted'
  Check 'at 150%, a restart keeps the size and place' (Same $cAfter $cBefore) "client $(Fmt $cBefore) -> $(Fmt $cAfter)"
} else {
  Log "Not at 150% ($dpi dpi): the scaling checks were skipped."
}
StopAll
Log "Back to 100%: $([W]::SetScale(100))"

Log "`n$script:failures failure(s)"
if ($script:failures) { exit 1 }
