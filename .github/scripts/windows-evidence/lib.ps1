# Shared helpers for the Windows evidence scripts: Win32 calls, screenshots, UI Automation and the
# check recorder. Dot-source it: . "$PSScriptRoot/lib.ps1"
#
# Written for Windows PowerShell 5.1 (`powershell.exe`), which ships UIAutomationClient and can
# read WinRT settings. The C# below stays within C# 5 for the same reason.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

if (-not ('TouchDeckEvidence.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace TouchDeckEvidence {
    public static class Native {
        [StructLayout(LayoutKind.Sequential)]
        public struct RECT { public int Left, Top, Right, Bottom; }
        [StructLayout(LayoutKind.Sequential)]
        public struct POINT { public int X, Y; }
        [StructLayout(LayoutKind.Sequential)]
        public struct NOTIFYICONIDENTIFIER {
            public uint cbSize; public IntPtr hWnd; public uint uID; public Guid guidItem;
        }
        [StructLayout(LayoutKind.Sequential)]
        public struct HIGHCONTRAST { public uint cbSize; public uint dwFlags; public IntPtr lpszDefaultScheme; }
        [StructLayout(LayoutKind.Sequential)]
        public struct MOUSEINPUT {
            public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr dwExtraInfo;
        }
        [StructLayout(LayoutKind.Sequential)]
        public struct INPUT { public uint type; public MOUSEINPUT mi; }

        delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lparam);

        [DllImport("shell32.dll")]
        static extern int Shell_NotifyIconGetRect(ref NOTIFYICONIDENTIFIER id, out RECT rect);
        [DllImport("user32.dll")]
        static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lparam);
        [DllImport("user32.dll")]
        public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll")]
        public static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll")]
        public static extern bool IsWindow(IntPtr hwnd);
        [DllImport("user32.dll")]
        public static extern bool IsIconic(IntPtr hwnd);
        [DllImport("user32.dll")]
        public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")]
        public static extern bool SetForegroundWindow(IntPtr hwnd);
        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr wparam, IntPtr lparam);
        [DllImport("user32.dll", SetLastError = true)]
        static extern IntPtr SendMessageTimeoutW(IntPtr hwnd, uint msg, IntPtr wparam, IntPtr lparam,
            uint flags, uint timeout, out IntPtr result);
        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true, EntryPoint = "SendMessageTimeoutW")]
        static extern IntPtr SendMessageTimeoutText(IntPtr hwnd, uint msg, IntPtr wparam, string lparam,
            uint flags, uint timeout, out IntPtr result);
        [DllImport("user32.dll")]
        public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
        [DllImport("user32.dll")]
        public static extern bool GetClientRect(IntPtr hwnd, out RECT rect);
        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
        [DllImport("user32.dll")]
        public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")]
        public static extern bool GetCursorPos(out POINT point);
        [DllImport("user32.dll", SetLastError = true)]
        static extern uint SendInput(uint count, INPUT[] inputs, int size);
        [DllImport("user32.dll")]
        public static extern uint GetSysColor(int index);
        [DllImport("user32.dll")]
        public static extern int GetSystemMetrics(int index);
        [DllImport("user32.dll")]
        public static extern uint GetDpiForSystem();
        [DllImport("user32.dll")]
        public static extern bool SetProcessDpiAwarenessContext(IntPtr context);
        [DllImport("user32.dll", SetLastError = true)]
        static extern bool SystemParametersInfoW(uint action, uint param, ref HIGHCONTRAST contrast, uint flags);
        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true, EntryPoint = "SystemParametersInfoW")]
        static extern bool SystemParametersInfoText(uint action, uint param, string text, uint flags);
        [DllImport("user32.dll")]
        public static extern int GetMenuItemCount(IntPtr menu);
        [DllImport("user32.dll")]
        public static extern uint GetMenuState(IntPtr menu, uint item, uint flags);
        [DllImport("user32.dll")]
        public static extern uint GetMenuDefaultItem(IntPtr menu, uint byPosition, uint flags);
        [DllImport("user32.dll")]
        public static extern uint GetMenuItemID(IntPtr menu, int position);
        [DllImport("dwmapi.dll")]
        static extern int DwmGetWindowAttribute(IntPtr hwnd, uint attribute, out RECT value, int size);
        [DllImport("dwmapi.dll")]
        static extern int DwmGetWindowAttribute(IntPtr hwnd, uint attribute, out int value, int size);
        [DllImport("dwmapi.dll")]
        static extern int DwmIsCompositionEnabled(out bool enabled);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern IntPtr LoadLibraryExW(string path, IntPtr file, uint flags);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern IntPtr FindResourceW(IntPtr module, IntPtr name, IntPtr type);
        [DllImport("kernel32.dll")]
        static extern bool FreeLibrary(IntPtr module);

        static string ClassOf(IntPtr hwnd) {
            StringBuilder text = new StringBuilder(256);
            GetClassNameW(hwnd, text, text.Capacity);
            return text.ToString();
        }
        public static string ClassName(IntPtr hwnd) { return ClassOf(hwnd); }
        public static string Title(IntPtr hwnd) {
            StringBuilder text = new StringBuilder(512);
            GetWindowTextW(hwnd, text, text.Capacity);
            return text.ToString();
        }
        public static uint ProcessOf(IntPtr hwnd) {
            uint pid;
            GetWindowThreadProcessId(hwnd, out pid);
            return pid;
        }

        // Every top-level window of a process (0 means any process), visible or not.
        public static IntPtr[] TopLevelWindows(uint pid) {
            List<IntPtr> found = new List<IntPtr>();
            EnumWindows(delegate(IntPtr hwnd, IntPtr unused) {
                if (pid == 0 || ProcessOf(hwnd) == pid) found.Add(hwnd);
                return true;
            }, IntPtr.Zero);
            return found.ToArray();
        }

        // The icon's rectangle as the notification area reports it. Returns the HRESULT.
        public static int NotifyIconRect(Guid guid, IntPtr hwnd, out RECT rect) {
            NOTIFYICONIDENTIFIER id = new NOTIFYICONIDENTIFIER();
            id.cbSize = (uint)Marshal.SizeOf(typeof(NOTIFYICONIDENTIFIER));
            id.hWnd = hwnd;
            id.guidItem = guid;
            return Shell_NotifyIconGetRect(ref id, out rect);
        }

        // DWMWA_EXTENDED_FRAME_BOUNDS: the window as drawn, without the invisible resize border.
        public static bool FrameBounds(IntPtr hwnd, out RECT rect) {
            if (DwmGetWindowAttribute(hwnd, 9, out rect, Marshal.SizeOf(typeof(RECT))) == 0) return true;
            return GetWindowRect(hwnd, out rect);
        }
        // DWMWA_CLOAKED: nonzero when DWM hides a window that still counts as visible.
        public static bool Cloaked(IntPtr hwnd) {
            int value;
            return DwmGetWindowAttribute(hwnd, 14, out value, 4) == 0 && value != 0;
        }
        public static bool Composition() {
            bool enabled;
            return DwmIsCompositionEnabled(out enabled) == 0 && enabled;
        }

        public static bool HighContrast(out uint flags, out string scheme) {
            HIGHCONTRAST contrast = new HIGHCONTRAST();
            contrast.cbSize = (uint)Marshal.SizeOf(typeof(HIGHCONTRAST));
            bool ok = SystemParametersInfoW(0x42, contrast.cbSize, ref contrast, 0);
            flags = contrast.dwFlags;
            scheme = contrast.lpszDefaultScheme == IntPtr.Zero ? null : Marshal.PtrToStringUni(contrast.lpszDefaultScheme);
            return ok;
        }
        // SPI_SETHIGHCONTRAST with SPIF_UPDATEINIFILE | SPIF_SENDCHANGE: what Settings and the
        // Left Alt + Left Shift + Print Screen shortcut do.
        public static bool SetHighContrast(uint flags, string scheme) {
            HIGHCONTRAST contrast = new HIGHCONTRAST();
            contrast.cbSize = (uint)Marshal.SizeOf(typeof(HIGHCONTRAST));
            contrast.dwFlags = flags;
            contrast.lpszDefaultScheme = scheme == null ? IntPtr.Zero : Marshal.StringToHGlobalUni(scheme);
            try { return SystemParametersInfoW(0x43, contrast.cbSize, ref contrast, 3); }
            finally { if (contrast.lpszDefaultScheme != IntPtr.Zero) Marshal.FreeHGlobal(contrast.lpszDefaultScheme); }
        }
        // SPI_SETDESKWALLPAPER, saved and broadcast.
        public static bool SetWallpaper(string path) {
            return SystemParametersInfoText(0x14, 0, path, 3);
        }

        // WM_SETTINGCHANGE to every top-level window, as Settings sends after it writes a value.
        public static bool BroadcastSettingChange(string area) {
            IntPtr result;
            return SendMessageTimeoutText(new IntPtr(0xffff), 0x1a, IntPtr.Zero, area, 2, 5000, out result) != IntPtr.Zero;
        }
        public static IntPtr Send(IntPtr hwnd, uint msg, IntPtr wparam, IntPtr lparam) {
            IntPtr result;
            SendMessageTimeoutW(hwnd, msg, wparam, lparam, 2, 5000, out result);
            return result;
        }

        // A real left click at a screen point, through the input queue like a mouse.
        public static bool Click(int x, int y) {
            if (!SetCursorPos(x, y)) return false;
            System.Threading.Thread.Sleep(150);
            INPUT[] inputs = new INPUT[2];
            inputs[0].type = 0; inputs[0].mi.dwFlags = 0x0002;
            inputs[1].type = 0; inputs[1].mi.dwFlags = 0x0004;
            return SendInput(2, inputs, Marshal.SizeOf(typeof(INPUT))) == 2;
        }

        // Whether the file carries a resource of this type and number (14 icon group, 16 version,
        // 24 manifest).
        public static bool HasResource(string path, int type, int id) {
            IntPtr module = LoadLibraryExW(path, IntPtr.Zero, 0x22);
            if (module == IntPtr.Zero) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
            try { return FindResourceW(module, new IntPtr(id), new IntPtr(type)) != IntPtr.Zero; }
            finally { FreeLibrary(module); }
        }
    }
}
'@
}

# Screen coordinates below are physical pixels: make this process DPI aware before it measures
# anything. It fails harmlessly when the host already chose.
[void][TouchDeckEvidence.Native]::SetProcessDpiAwarenessContext([IntPtr]-4)

# ---- files -------------------------------------------------------------------------------------

function Write-Utf8([string]$Path, [string]$Text) {
    [IO.File]::WriteAllText($Path, $Text, (New-Object Text.UTF8Encoding $false))
}

function Write-Json([string]$Path, $Value) {
    Write-Utf8 $Path (($Value | ConvertTo-Json -Depth 12) + "`n")
}

function Wait-Until([scriptblock]$Condition, [double]$Seconds = 10, [int]$EveryMs = 200) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    do {
        $value = & $Condition
        if ($value) { return $value }
        Start-Sleep -Milliseconds $EveryMs
    } while ([DateTime]::UtcNow -lt $deadline)
    return $null
}

# ---- checks ------------------------------------------------------------------------------------

$script:Checks = New-Object System.Collections.Generic.List[object]

# Records one pass-or-fail check. A check that throws fails with the error as its detail.
function Add-Check([string]$Id, [string]$Name, [scriptblock]$Test) {
    $passed = $false
    $detail = ''
    try {
        $result = & $Test
        if ($result -is [bool]) { $passed = $result }
        else {
            $passed = [bool]$result.pass
            $detail = [string]$result.detail
        }
    } catch {
        $detail = "error: $($_.Exception.Message) ($($_.InvocationInfo.ScriptLineNumber))"
    }
    $script:Checks.Add([ordered]@{ id = $Id; name = $Name; pass = $passed; detail = $detail })
    Write-Host ("[{0}] {1} {2}{3}" -f $(if ($passed) { 'pass' } else { 'FAIL' }), $Id, $Name, $(if ($detail) { " - $detail" } else { '' }))
}

function Get-Checks { , $script:Checks.ToArray() }

# ---- windows -----------------------------------------------------------------------------------

function Get-Rect([IntPtr]$Hwnd) {
    $rect = New-Object TouchDeckEvidence.Native+RECT
    [void][TouchDeckEvidence.Native]::FrameBounds($Hwnd, [ref]$rect)
    [ordered]@{ x = $rect.Left; y = $rect.Top; width = $rect.Right - $rect.Left; height = $rect.Bottom - $rect.Top }
}

function Test-Shown([IntPtr]$Hwnd) {
    [TouchDeckEvidence.Native]::IsWindowVisible($Hwnd) -and -not [TouchDeckEvidence.Native]::Cloaked($Hwnd) -and
        -not [TouchDeckEvidence.Native]::IsIconic($Hwnd)
}

# The app's main window: the process's top-level window titled "Open TouchDeck" that is not the
# hidden tray window. It exists while hidden too.
function Get-AppWindow([int]$ProcessId) {
    foreach ($hwnd in [TouchDeckEvidence.Native]::TopLevelWindows([uint32]$ProcessId)) {
        if ([TouchDeckEvidence.Native]::Title($hwnd) -eq 'Open TouchDeck' -and
            [TouchDeckEvidence.Native]::ClassName($hwnd) -ne 'OpenTouchDeckTray') { return $hwnd }
    }
    return [IntPtr]::Zero
}

function Get-WindowOfClass([int]$ProcessId, [string]$Class) {
    foreach ($hwnd in [TouchDeckEvidence.Native]::TopLevelWindows([uint32]$ProcessId)) {
        if ([TouchDeckEvidence.Native]::ClassName($hwnd) -eq $Class) { return $hwnd }
    }
    return [IntPtr]::Zero
}

# ---- screenshots -------------------------------------------------------------------------------

# Copies a screen rectangle as the user sees it, DWM effects included, into a PNG.
function Save-ScreenRect([string]$Path, [int]$X, [int]$Y, [int]$Width, [int]$Height) {
    $bitmap = New-Object Drawing.Bitmap $Width, $Height
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($X, $Y, 0, 0, $bitmap.Size)
        $bitmap.Save($Path, [Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

function Save-Screen([string]$Path) {
    $bounds = [Windows.Forms.SystemInformation]::VirtualScreen
    Save-ScreenRect $Path $bounds.X $bounds.Y $bounds.Width $bounds.Height
}

function Save-Window([string]$Path, [IntPtr]$Hwnd) {
    $rect = Get-Rect $Hwnd
    $screen = [Windows.Forms.SystemInformation]::VirtualScreen
    $left = [Math]::Max($rect.x, $screen.X)
    $top = [Math]::Max($rect.y, $screen.Y)
    $right = [Math]::Min($rect.x + $rect.width, $screen.Right)
    $bottom = [Math]::Min($rect.y + $rect.height, $screen.Bottom)
    Save-ScreenRect $Path $left $top ($right - $left) ($bottom - $top)
}

# The colour of a few pixels of a saved screenshot, as #rrggbb, and its mean brightness (0-255).
function Get-ImageFacts([string]$Path, [object[]]$Points) {
    $bitmap = New-Object Drawing.Bitmap $Path
    try {
        $sum = 0.0
        $count = 0
        for ($y = 0; $y -lt $bitmap.Height; $y += 8) {
            for ($x = 0; $x -lt $bitmap.Width; $x += 8) {
                $pixel = $bitmap.GetPixel($x, $y)
                $sum += 0.2126 * $pixel.R + 0.7152 * $pixel.G + 0.0722 * $pixel.B
                $count++
            }
        }
        $samples = [ordered]@{}
        foreach ($point in $Points) {
            $x = [Math]::Min([int]$point[1], $bitmap.Width - 1)
            $y = [Math]::Min([int]$point[2], $bitmap.Height - 1)
            $pixel = $bitmap.GetPixel($x, $y)
            $samples[$point[0]] = '#{0:x2}{1:x2}{2:x2}' -f $pixel.R, $pixel.G, $pixel.B
        }
        [ordered]@{ width = $bitmap.Width; height = $bitmap.Height; meanBrightness = [Math]::Round($sum / $count, 1); pixels = $samples }
    } finally { $bitmap.Dispose() }
}

# ---- UI Automation -----------------------------------------------------------------------------

function Get-UiaProperty($Element, [string]$Name) {
    try { $Element.Current.$Name } catch { $null }
}

# One element as plain data.
function ConvertTo-UiaNode($Element, [int]$Depth) {
    $rect = Get-UiaProperty $Element 'BoundingRectangle'
    $type = Get-UiaProperty $Element 'ControlType'
    $patterns = @()
    try { $patterns = @($Element.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName -replace 'PatternIdentifiers\.Pattern$', '' }) } catch { }
    $node = [ordered]@{
        depth       = $Depth
        type        = $(if ($type) { $type.ProgrammaticName -replace '^ControlType\.', '' } else { '' })
        name        = [string](Get-UiaProperty $Element 'Name')
        description = [string](Get-UiaProperty $Element 'HelpText')
        automationId = [string](Get-UiaProperty $Element 'AutomationId')
        class       = [string](Get-UiaProperty $Element 'ClassName')
        focusable   = [bool](Get-UiaProperty $Element 'IsKeyboardFocusable')
        focused     = [bool](Get-UiaProperty $Element 'HasKeyboardFocus')
        enabled     = [bool](Get-UiaProperty $Element 'IsEnabled')
        offscreen   = [bool](Get-UiaProperty $Element 'IsOffscreen')
        control     = [bool](Get-UiaProperty $Element 'IsControlElement')
        bounds      = $(if ($rect -and -not $rect.IsEmpty) { '{0},{1} {2}x{3}' -f [int]$rect.X, [int]$rect.Y, [int]$rect.Width, [int]$rect.Height } else { '' })
        patterns    = $patterns
    }
    try {
        $pattern = $null
        if ($Element.TryGetCurrentPattern([Windows.Automation.TogglePattern]::Pattern, [ref]$pattern)) {
            $node.toggle = [string]$pattern.Current.ToggleState
        }
        if ($Element.TryGetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern, [ref]$pattern)) {
            $node.selected = [bool]$pattern.Current.IsSelected
        }
    } catch { }
    $node
}

# The whole tree under an element in the raw view (what Narrator can walk), depth first.
function Get-UiaTree($Root, [int]$MaxNodes = 4000) {
    $walker = [Windows.Automation.TreeWalker]::RawViewWalker
    $nodes = New-Object System.Collections.Generic.List[object]
    $stack = New-Object System.Collections.Stack
    $stack.Push(@($Root, 0))
    while ($stack.Count -gt 0 -and $nodes.Count -lt $MaxNodes) {
        $entry = $stack.Pop()
        $element = $entry[0]
        $depth = $entry[1]
        $nodes.Add((ConvertTo-UiaNode $element $depth))
        $children = New-Object System.Collections.Generic.List[object]
        try {
            $child = $walker.GetFirstChild($element)
            while ($child -and $children.Count -lt 500) {
                $children.Add($child)
                $child = $walker.GetNextSibling($child)
            }
        } catch { }
        for ($i = $children.Count - 1; $i -ge 0; $i--) { $stack.Push(@($children[$i], $depth + 1)) }
    }
    , $nodes.ToArray()
}

function Format-UiaTree($Nodes) {
    $lines = foreach ($node in $Nodes) {
        $parts = @($node.type)
        $parts += '"{0}"' -f $node.name
        if ($node.description) { $parts += 'description="{0}"' -f $node.description }
        if ($node.focusable) { $parts += 'focusable' }
        if ($node.focused) { $parts += 'focused' }
        if (-not $node.enabled) { $parts += 'disabled' }
        if ($node.offscreen) { $parts += 'offscreen' }
        if ($node.Contains('selected')) { $parts += "selected=$($node.selected)" }
        if ($node.Contains('toggle')) { $parts += "toggle=$($node.toggle)" }
        if ($node.patterns.Count) { $parts += '[{0}]' -f ($node.patterns -join ',') }
        if ($node.bounds) { $parts += "@ $($node.bounds)" }
        ('  ' * $node.depth) + ($parts -join ' ')
    }
    ($lines -join "`n") + "`n"
}

function Get-UiaElement([IntPtr]$Hwnd) {
    [Windows.Automation.AutomationElement]::FromHandle($Hwnd)
}

# The first descendant with this name and, when given, control type ("Button", "TabItem", ...).
function Find-UiaByName($Root, [string]$Name, [string]$Type) {
    $condition = New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::NameProperty), $Name
    foreach ($element in $Root.FindAll([Windows.Automation.TreeScope]::Descendants, $condition)) {
        if (-not $Type -or $element.Current.ControlType.ProgrammaticName -eq "ControlType.$Type") { return $element }
    }
    return $null
}

# Clicks the middle of an element with the mouse.
function Invoke-UiaClick($Element) {
    $rect = $Element.Current.BoundingRectangle
    if ($rect.IsEmpty) { throw "The element '$($Element.Current.Name)' has no place on screen." }
    $x = [int]($rect.X + $rect.Width / 2)
    $y = [int]($rect.Y + $rect.Height / 2)
    if (-not [TouchDeckEvidence.Native]::Click($x, $y)) { throw "Could not click at $x,$y." }
}
