<#
.SYNOPSIS
Proves the notification-area icon's behaviour on a real desktop: eight pass-or-fail checks in
tray.json, with screenshots of the notification area.

.DESCRIPTION
The script stands in for the Shell where it must. A version 4 notification icon gets its events
as its callback message (WM_APP+1) with the event in the low word of lParam, so the script posts
that message to the hidden OpenTouchDeckTray window: NIN_SELECT for a left click, NIN_KEYSELECT
for Enter or Space, WM_CONTEXTMENU for a right click. Everything after that is the app's own
code and real Windows: the popup menu is read through UI Automation and clicked with the mouse.

  1 The icon is present              Shell_NotifyIconGetRect succeeds for the GUID
  2 Left-click opens                 NIN_SELECT and NIN_KEYSELECT show the hidden window, in front
  3 Menu                             exactly Open (default), Start at sign-in, Quit; Open opens
  4 Quit exits                       exit code 0 within 7 s and the icon's rectangle is gone
  5 Start at sign-in                 the Run value and the StartupApproved record, with --packaged
  6 Close keeps running              after the window closes, the process and the icon stay
  7 Explorer restart                 the icon comes back after Explorer restarts
  8 Identity                         the window's AppUserModelID, the icons and the version resource

Check 5 writes this user's real Run and StartupApproved values and check 7 restarts Explorer.
Run it on a machine you can throw away.

.EXAMPLE
powershell -File .github/scripts/windows-evidence/tray.ps1 -Exe app/target/release/td-app.exe -Out evidence
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Out,
    # td-app's Cargo version. Read from its Cargo.toml when omitted.
    [string]$Version
)
. "$PSScriptRoot/lib.ps1"

$Exe = (Resolve-Path -LiteralPath $Exe).Path
$Out = (New-Item -ItemType Directory -Force -Path $Out).FullName
$shots = (New-Item -ItemType Directory -Force -Path (Join-Path $Out 'screenshots')).FullName
$uia = (New-Item -ItemType Directory -Force -Path (Join-Path $Out 'uia')).FullName
if (-not $Version) {
    $manifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot '../../../app/crates/td-app/Cargo.toml') -Raw
    if ($manifest -notmatch '(?m)^\[package\]\r?\n(?:(?!\[)[^\n]*\n)*?version\s*=\s*"([^"]+)"') { throw 'td-app/Cargo.toml has no package version.' }
    $Version = $Matches[1]
}

# The values td-companion's tray code uses (app/crates/td-companion/src/tray/windows.rs).
$TrayGuid = [Guid]'b48d722e-6be7-4f73-a636-cff1b93761c7'
$TrayClass = 'OpenTouchDeckTray'
$Callback = 0x8001          # WM_APP + 1
$NIN_SELECT = 0x0400
$NIN_KEYSELECT = 0x0401
$WM_CONTEXTMENU = 0x007b
$AppId = 'io.github.duresa7.touchdeck'
$RunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$ApprovedKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
$RunName = 'Open TouchDeck'

$facts = [ordered]@{}
$app = $null

# ---- the icon ----------------------------------------------------------------------------------

function Get-IconRect {
    $rect = New-Object TouchDeckEvidence.Native+RECT
    $result = [TouchDeckEvidence.Native]::NotifyIconRect($TrayGuid, [IntPtr]::Zero, [ref]$rect)
    [ordered]@{
        found = $result -eq 0; hresult = '0x{0:x8}' -f $result
        x = $rect.Left; y = $rect.Top; width = $rect.Right - $rect.Left; height = $rect.Bottom - $rect.Top
    }
}

function Test-Icon { (Get-IconRect).found }

# Posts one Shell callback to the tray window, with the screen point packed as version 4 does.
function Send-TrayEvent([IntPtr]$Tray, [int]$Code, [int]$X, [int]$Y) {
    $point = ($X -band 0xffff) -bor (($Y -band 0xffff) -shl 16)
    if (-not [TouchDeckEvidence.Native]::PostMessageW($Tray, $Callback, [IntPtr]$point, [IntPtr]$Code)) {
        throw 'Could not post the callback to the tray window.'
    }
}

# ---- the menu ----------------------------------------------------------------------------------

# Opens the tray menu with a WM_CONTEXTMENU callback and reads it: the names through UI
# Automation, and each item's checked, disabled and default state from the menu itself.
function Open-TrayMenu([IntPtr]$Tray, [int]$ProcessId) {
    $icon = Get-IconRect
    Send-TrayEvent $Tray $WM_CONTEXTMENU $icon.x $icon.y
    $window = Wait-Until { Get-WindowOfClass $ProcessId '#32768' -Visible } 6
    if ($null -eq $window) { throw 'The tray menu did not open.' }
    Start-Sleep -Milliseconds 500
    # MN_GETHMENU
    $menu = [TouchDeckEvidence.Native]::Send($window, 0x01e1, [IntPtr]::Zero, [IntPtr]::Zero)
    $elements = @([TouchDeckEvidence.Menus]::Items($window))
    $default = [TouchDeckEvidence.Native]::GetMenuDefaultItem($menu, 1, 1)
    $items = @(for ($i = 0; $i -lt $elements.Count; $i++) {
            # MF_BYPOSITION; MF_CHECKED is 8, MF_GRAYED 1 and MF_DISABLED 2.
            $state = [TouchDeckEvidence.Native]::GetMenuState($menu, [uint32]$i, 0x400)
            [ordered]@{
                name = $elements[$i].Name; checked = ($state -band 8) -ne 0
                enabled = ($state -band 3) -eq 0; default = $default -eq $i
                # The same state as UI Automation reports it, for comparison.
                checkedInUiAutomation = ($elements[$i].State -band 0x10) -ne 0
            }
        })
    [ordered]@{ window = $window; elements = $elements; items = $items; count = [TouchDeckEvidence.Native]::GetMenuItemCount($menu) }
}

function Format-Menu($Menu) {
    ($Menu.items | ForEach-Object {
        $_.name + $(if ($_.default) { ' (default)' } else { '' }) + $(if ($_.checked) { ' (checked)' } else { '' }) + $(if (-not $_.enabled) { ' (disabled)' } else { '' })
    }) -join ', '
}

# Dismisses the menu without choosing: WM_CANCELMODE to its owner.
function Close-TrayMenu([IntPtr]$Tray, $Menu) {
    [void][TouchDeckEvidence.Native]::Send($Tray, 0x001f, [IntPtr]::Zero, [IntPtr]::Zero)
    [void](Wait-Until { -not [TouchDeckEvidence.Native]::IsWindow($Menu.window) -or -not [TouchDeckEvidence.Native]::IsWindowVisible($Menu.window) } 5)
    Start-Sleep -Milliseconds 300
}

# Chooses a menu item with a mouse click.
function Select-TrayMenuItem($Menu, [string]$Name) {
    $index = 0
    foreach ($item in $Menu.items) { if ($item.name -eq $Name) { break }; $index++ }
    if ($index -ge $Menu.items.Count) { throw "The menu has no item named $Name." }
    $element = $Menu.elements[$index]
    if (-not [TouchDeckEvidence.Native]::Click($element.X + [int]($element.Width / 2), $element.Y + [int]($element.Height / 2))) { throw "Could not click $Name." }
    $closed = Wait-Until { -not [TouchDeckEvidence.Native]::IsWindow($Menu.window) -or -not [TouchDeckEvidence.Native]::IsWindowVisible($Menu.window) } 5
    if (-not $closed) { throw "Clicking $Name did not close the menu." }
    Start-Sleep -Milliseconds 300
}

function Get-RunValue {
    try { (Get-ItemProperty -LiteralPath $RunKey -Name $RunName -ErrorAction Stop).$RunName } catch { $null }
}

function Get-ApprovedState {
    try {
        $bytes = (Get-ItemProperty -LiteralPath $ApprovedKey -Name $RunName -ErrorAction Stop).$RunName
        [BitConverter]::ToUInt32($bytes, 0)
    } catch { $null }
}

function Save-NotificationArea([string]$Label) {
    $taskbar = Get-WindowOfClass 0 'Shell_TrayWnd'
    if ($null -eq $taskbar) { return $null }
    $rect = Get-Rect $taskbar
    Save-ScreenRect (Join-Path $shots "$Label-taskbar.png") $rect.x $rect.y $rect.width $rect.height
    $nodes = Get-UiaTree (Get-UiaElement $taskbar)
    Write-Utf8 (Join-Path $uia "$Label-taskbar.txt") (Format-UiaTree $nodes)
    # The icon's button is named by its tooltip. The app's taskbar button has a longer name.
    $onTaskbar = @($nodes | Where-Object { $_.name -eq 'Open TouchDeck' -and -not $_.offscreen }).Count -gt 0
    $place = if ($onTaskbar) { 'taskbar' } else { 'not found' }
    if (-not $onTaskbar) {
        # Windows 11 puts a new icon in the overflow flyout. Open it as a user does.
        $chevron = Find-UiaByName (Get-UiaElement $taskbar) 'Show Hidden Icons' 'Button'
        if ($chevron) {
            Invoke-UiaClick $chevron
            Start-Sleep -Seconds 2
            Save-Screen (Join-Path $shots "$Label-overflow.png")
            $overflow = Get-WindowOfClass 0 'TopLevelWindowForOverflowXamlIsland' -Visible
            if ($null -ne $overflow) {
                $inside = Get-UiaTree (Get-UiaElement $overflow)
                Write-Utf8 (Join-Path $uia "$Label-overflow.txt") (Format-UiaTree $inside)
                if (@($inside | Where-Object { $_.name -eq 'Open TouchDeck' }).Count) { $place = 'overflow flyout' }
            }
            Invoke-UiaClick $chevron
            Start-Sleep -Seconds 1
        }
    }
    $place
}

try {
    Hide-OtherWindows
    # ============================ a mock run: checks 1, 6, 2, 3, 8, 4 ============================
    $data = Join-Path ([IO.Path]::GetTempPath()) "td-evidence-tray-$PID"
    $app = Start-App $Exe @('--mock-devices', '1') $data (Join-Path $Out 'logs/tray-stderr.txt')
    $hwnd = Wait-AppWindow $app
    $tray = Wait-Until { Get-WindowOfClass $app.Id $TrayClass } 10
    if ($null -eq $tray) { throw "The app has no $TrayClass window." }
    [void](Enter-Window $hwnd)

    Add-Check 'tray-1' 'The icon is present' {
        $icon = Wait-Until { $rect = Get-IconRect; if ($rect.found) { $rect } } 10
        $facts.icon = if ($icon) { $icon } else { Get-IconRect }
        @{ pass = [bool]$icon; detail = "Shell_NotifyIconGetRect for {$TrayGuid}: $($facts.icon.hresult), $($facts.icon.width)x$($facts.icon.height) at $($facts.icon.x),$($facts.icon.y)" }
    }
    Save-Screen (Join-Path $shots 'tray-desktop.png')
    $facts.iconPlace = Save-NotificationArea 'tray'

    Add-Check 'tray-6' 'Close keeps running' {
        $closed = Hide-ToTray $hwnd {
            Save-Window (Join-Path $shots 'close-notice.png') $hwnd
            Write-Utf8 (Join-Path $uia 'close-notice.txt') (Format-UiaTree (Get-UiaTree (Get-UiaElement $hwnd)))
        }
        Start-Sleep -Seconds 3
        $alive = -not $app.HasExited
        $icon = Test-Icon
        @{
            pass   = $closed.notice -and $closed.hidden -and $alive -and $icon
            detail = "one-time notice shown: $($closed.notice); window hidden: $($closed.hidden); process running 3 s later: $alive; icon present: $icon"
        }
    }

    Add-Check 'tray-2' 'Left-click opens' {
        $results = foreach ($kind in @(@('NIN_SELECT', $NIN_SELECT), @('NIN_KEYSELECT', $NIN_KEYSELECT))) {
            if (Test-Shown $hwnd) {
                $again = Hide-ToTray $hwnd $null
                if ($again.notice) { throw 'The close notice showed a second time.' }
            }
            $hiddenBefore = -not (Test-Shown $hwnd)
            $icon = Get-IconRect
            Send-TrayEvent $tray $kind[1] $icon.x $icon.y
            $shown = [bool](Wait-Until { Test-Shown $hwnd } 5)
            $front = [bool](Wait-Until { [TouchDeckEvidence.Native]::GetForegroundWindow() -eq $hwnd } 3)
            [ordered]@{ event = $kind[0]; hiddenBefore = $hiddenBefore; shown = $shown; front = $front }
        }
        $facts.open = @($results)
        @{
            pass   = @($results | Where-Object { -not ($_.hiddenBefore -and $_.shown -and $_.front) }).Count -eq 0
            detail = ($results | ForEach-Object { "$($_.event): hidden before $($_.hiddenBefore), visible $($_.shown), in front $($_.front)" }) -join '; '
        }
    }

    Add-Check 'tray-3' 'Menu' {
        [void](Hide-ToTray $hwnd $null)
        $menu = Open-TrayMenu $tray $app.Id
        Save-Screen (Join-Path $shots 'tray-menu.png')
        Write-Json (Join-Path $uia 'tray-menu.json') $menu.elements
        $facts.menu = $menu.items
        $names = @($menu.items | ForEach-Object { $_.name })
        $exact = ($names -join '|') -ceq 'Open|Start at sign-in|Quit' -and $menu.count -eq 3
        $defaults = @($menu.items | Where-Object { $_.default } | ForEach-Object { $_.name })
        Select-TrayMenuItem $menu 'Open'
        $opened = [bool](Wait-Until { (Test-Shown $hwnd) -and [TouchDeckEvidence.Native]::GetForegroundWindow() -eq $hwnd } 5)
        @{
            pass   = $exact -and ($defaults -join '|') -ceq 'Open' -and $opened
            detail = "items read through UI Automation: $(Format-Menu $menu); choosing Open showed the window in front: $opened"
        }
    }

    Add-Check 'tray-8' 'Identity' {
        # In a process of its own: see aumid.ps1. Its stderr must not stop this script.
        $ErrorActionPreference = 'Continue'
        $resolved = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'aumid.ps1') -Hwnd $hwnd.ToInt64() 2>&1 | ForEach-Object { "$_" })
        $resolverExit = $LASTEXITCODE
        $ErrorActionPreference = 'Stop'
        $id = $null
        if ($resolverExit -eq 0) { $id = (($resolved | Select-Object -Last 1) | ConvertFrom-Json) }
        $info = [Diagnostics.FileVersionInfo]::GetVersionInfo($Exe)
        $bytes = [IO.File]::ReadAllBytes($Exe)
        $pe = [BitConverter]::ToInt32($bytes, 0x3c)
        $subsystem = [BitConverter]::ToUInt16($bytes, $pe + 24 + 68)
        $resources = [ordered]@{
            appIcon  = [TouchDeckEvidence.Native]::HasResource($Exe, 14, 1)
            trayIcon = [TouchDeckEvidence.Native]::HasResource($Exe, 14, 2)
            manifest = [TouchDeckEvidence.Native]::HasResource($Exe, 24, 1)
            version  = [TouchDeckEvidence.Native]::HasResource($Exe, 16, 1)
        }
        $icon = [Drawing.Icon]::ExtractAssociatedIcon($Exe)
        $icon.ToBitmap().Save((Join-Path $shots 'exe-icon.png'), [Drawing.Imaging.ImageFormat]::Png)
        $facts.identity = [ordered]@{
            appUserModelId = $(if ($id) { $id.appUserModelId } else { $null }); explicit = $(if ($id) { $id.explicit } else { $null })
            resolver = $(if ($resolverExit -eq 0) { 'IApplicationResolver.GetAppIDForWindow' } else { "failed ($resolverExit): $($resolved -join ' ')" })
            subsystem = $(switch ($subsystem) { 2 { 'gui' } 3 { 'console' } default { "$subsystem" } })
            resources = $resources
            ProductName = $info.ProductName; FileDescription = $info.FileDescription; CompanyName = $info.CompanyName
            OriginalFilename = $info.OriginalFilename; InternalName = $info.InternalName; LegalCopyright = $info.LegalCopyright
            ProductVersion = $info.ProductVersion; FileVersion = $info.FileVersion
        }
        $expected = [ordered]@{
            appUserModelId = $AppId; explicit = $true; subsystem = 'gui'; ProductName = 'Open TouchDeck'; FileDescription = 'Open TouchDeck'
            CompanyName = 'Duresa Kadi'; OriginalFilename = 'td-app.exe'; InternalName = 'td-app'; LegalCopyright = 'Copyright Duresa Kadi'
            ProductVersion = $Version; FileVersion = $Version
        }
        $wrong = @(foreach ($name in $expected.Keys) {
                if ("$($facts.identity[$name])" -cne "$($expected[$name])") { "$name is '$($facts.identity[$name])', not '$($expected[$name])'" }
            })
        $wrong += @($resources.Keys | Where-Object { -not $resources[$_] } | ForEach-Object { "no $_ resource" })
        @{
            pass   = $wrong.Count -eq 0
            detail = $(if ($wrong.Count) { $wrong -join '; ' } else { "the shell resolves the window to $AppId (explicit); the exe carries the app icon (1), the tray icon (2), the manifest and version $Version" })
        }
    }

    Add-Check 'tray-4' 'Quit exits' {
        $menu = Open-TrayMenu $tray $app.Id
        $clock = [Diagnostics.Stopwatch]::StartNew()
        Select-TrayMenuItem $menu 'Quit'
        $exited = $app.WaitForExit(7000)
        $seconds = [Math]::Round($clock.Elapsed.TotalSeconds, 1)
        $code = if ($exited) { $app.ExitCode } else { $null }
        $gone = [bool](Wait-Until { -not (Test-Icon) } 5)
        $facts.quit = [ordered]@{ exited = $exited; seconds = $seconds; exitCode = $code; iconGone = $gone; iconAfter = (Get-IconRect).hresult }
        @{ pass = $exited -and $code -eq 0 -and $gone; detail = "exited: $exited after $seconds s with code $code; Shell_NotifyIconGetRect afterwards: $($facts.quit.iconAfter)" }
    }
    Stop-App $app

    # ======================== an installed-style run: checks 5 and 7 =============================
    # --packaged is the command line Start at sign-in registers. It runs the real Companion, so
    # the menu reads and writes the real registry values (a mock run keeps them in memory).
    Remove-ItemProperty -LiteralPath $RunKey -Name $RunName -ErrorAction SilentlyContinue
    Remove-ItemProperty -LiteralPath $ApprovedKey -Name $RunName -ErrorAction SilentlyContinue
    $data = Join-Path ([IO.Path]::GetTempPath()) "td-evidence-packaged-$PID"
    $app = Start-App $Exe @('--packaged') $data (Join-Path $Out 'logs/packaged-stderr.txt')
    $hwnd = Wait-AppWindow $app
    $tray = Wait-Until { Get-WindowOfClass $app.Id $TrayClass } 10
    if ($null -eq $tray) { throw "The app has no $TrayClass window." }
    [void](Wait-Until { Test-Icon } 10)

    Add-Check 'tray-5' 'Start at sign-in' {
        $steps = New-Object System.Collections.Generic.List[object]
        $read = {
            param([string]$Step)
            $menu = Open-TrayMenu $tray $app.Id
            $item = $menu.items | Where-Object { $_.name -eq 'Start at sign-in' }
            if ($Step -eq 'first run') { Save-Screen (Join-Path $shots 'tray-menu-start-at-sign-in.png') }
            $state = [ordered]@{ step = $Step; run = (Get-RunValue); approvedState = (Get-ApprovedState); checked = $item.checked; enabled = $item.enabled }
            $steps.Add($state)
            $menu
        }
        $expectedRun = '"{0}" --packaged' -f $app.MainModule.FileName

        # The first --packaged run registers the app and the item is checked.
        [void](Wait-Until { Get-RunValue } 10)
        $menu = & $read 'first run'
        $first = $steps[0].run -ceq $expectedRun -and $steps[0].checked
        Close-TrayMenu $tray $menu

        # Windows' Startup apps list turns an app off with a StartupApproved record whose state is 3.
        if (-not (Test-Path -LiteralPath $ApprovedKey)) { New-Item -Path $ApprovedKey -Force | Out-Null }
        $record = [byte[]](@(3, 0, 0, 0) + [BitConverter]::GetBytes([DateTime]::UtcNow.ToFileTimeUtc()))
        Set-ItemProperty -LiteralPath $ApprovedKey -Name $RunName -Value $record -Type Binary
        $menu = & $read 'disabled record written'
        $disabled = $steps[1].approvedState -eq 3 -and -not $steps[1].checked -and $steps[1].enabled

        # Choosing the unchecked item turns it on again: the record goes and the Run value stays.
        Select-TrayMenuItem $menu 'Start at sign-in'
        [void](Wait-Until { $null -eq (Get-ApprovedState) } 5)
        $menu = & $read 'chosen once'
        $cleared = $null -eq $steps[2].approvedState -and $steps[2].run -ceq $expectedRun -and $steps[2].checked

        # Choosing it once more turns it off: the Run value goes.
        Select-TrayMenuItem $menu 'Start at sign-in'
        [void](Wait-Until { $null -eq (Get-RunValue) } 5)
        $menu = & $read 'chosen twice'
        $removed = $null -eq $steps[3].run -and -not $steps[3].checked
        Close-TrayMenu $tray $menu

        $facts.startAtSignIn = [ordered]@{ expectedRun = '"<td-app.exe>" --packaged'; runMatchesExecutable = $steps[0].run -ceq $expectedRun; steps = $steps.ToArray() }
        # The Run value holds the runner's build path; keep the file name only.
        foreach ($step in $steps) { if ($step.run) { $step.run = $step.run -replace '^"[^"]*\\', '"<folder>\' } }
        @{
            pass   = $first -and $disabled -and $cleared -and $removed
            detail = "first --packaged run registers `"<td-app.exe>`" --packaged and shows checked: $first; " +
                "a state 3 record shows unchecked: $disabled; choosing it clears the record and checks it: $cleared; choosing it again removes the Run value: $removed"
        }
    }

    Add-Check 'tray-7' 'Explorer restart' {
        $before = @(Get-Process -Name explorer -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
        $iconBefore = Test-Icon
        Stop-Process -Name explorer -Force
        $lost = [bool](Wait-Until { -not (Test-Icon) } 5 100)
        $how = 'Windows restarted it'
        $restarted = Wait-Until { @(Get-Process -Name explorer -ErrorAction SilentlyContinue | Where-Object { $before -notcontains $_.Id }).Count -gt 0 } 8
        if (-not $restarted) {
            Start-Process -FilePath "$env:WINDIR\explorer.exe"
            $how = 'the script started it'
        }
        $taskbar = [bool](Wait-Until { $null -ne (Get-WindowOfClass 0 'Shell_TrayWnd') } 60)
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $back = [bool](Wait-Until { Test-Icon } 60)
        $seconds = [Math]::Round($clock.Elapsed.TotalSeconds, 1)
        Start-Sleep -Seconds 5
        Save-Screen (Join-Path $shots 'tray-after-explorer-restart.png')
        $place = Save-NotificationArea 'tray-after-explorer-restart'
        $facts.explorerRestart = [ordered]@{ iconBefore = $iconBefore; iconLostWhileExplorerWasDown = $lost; explorer = $how; taskbarBack = $taskbar; iconBack = $back; secondsAfterTaskbar = $seconds; iconPlace = $place; icon = (Get-IconRect) }
        @{
            pass   = $iconBefore -and $taskbar -and $back -and -not $app.HasExited
            detail = "Explorer was stopped and $how; the taskbar came back: $taskbar; Shell_NotifyIconGetRect succeeds again: $back ($seconds s after the taskbar); process still running: $(-not $app.HasExited)"
        }
    }
} finally {
    Stop-App $app
    Remove-ItemProperty -LiteralPath $RunKey -Name $RunName -ErrorAction SilentlyContinue
    $facts.checks = Get-Checks
    Write-Json (Join-Path $Out 'tray.json') $facts
}
