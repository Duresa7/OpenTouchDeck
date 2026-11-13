<#
.SYNOPSIS
Screenshots of td-app's Home and About pages in light, dark, a contrast theme and 200% text
size, one at the minimum width, and the UI Automation tree of both pages.

.DESCRIPTION
Runs td-app with two mock Devices and changes each Windows setting the way Settings does: the
registry value plus the WM_SETTINGCHANGE broadcast, or SystemParametersInfo. The app keeps
running across the changes, so the run also shows whether it follows them live. When a change is
not seen, the window is saved as <look>-not-followed.png, the app is restarted to get the
screenshots, and look.json says so.

Screenshots are copied from the screen, so they show what DWM composes, Mica included.

This changes the signed-in user's theme, contrast and text size and does not put them back. Run
it on a machine you can throw away.

.EXAMPLE
powershell -File .github/scripts/windows-evidence/look.ps1 -Exe app/target/release/td-app.exe -Out evidence
#>
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Exe, [Parameter(Mandatory)][string]$Out)
. "$PSScriptRoot/lib.ps1"

$Exe = (Resolve-Path -LiteralPath $Exe).Path
$Out = (New-Item -ItemType Directory -Force -Path $Out).FullName
$shots = (New-Item -ItemType Directory -Force -Path (Join-Path $Out 'screenshots')).FullName
$uia = (New-Item -ItemType Directory -Force -Path (Join-Path $Out 'uia')).FullName
$data = Join-Path ([IO.Path]::GetTempPath()) "td-evidence-look-$PID"
$facts = [ordered]@{ captures = [ordered]@{}; changes = [ordered]@{}; restarts = 0 }
$app = $null
$hwnd = $null
$starts = 0

function Start-Look {
    $script:starts++
    $script:app = Start-App $Exe @('--mock-devices', '2') $data (Join-Path $Out "logs/look-stderr-$script:starts.txt")
    $script:hwnd = Wait-AppWindow $script:app
    Start-Sleep -Seconds 3
}

function Restart-Look {
    Stop-App $script:app
    $facts.restarts++
    Start-Look
}

# Home and About as they are now.
function Save-Pages([string]$Label, [string[]]$Pages = @('Home', 'About')) {
    foreach ($page in $Pages) {
        $front = Enter-Window $hwnd
        $selected = Select-Page $hwnd $page
        $file = Join-Path $shots "$Label-$($page.ToLower()).png"
        Save-Window $file $hwnd
        $rect = Get-Rect $hwnd
        $image = Get-ImageFacts $file @(
            @('navigationPaneLow', 40, ($rect.height - 60)),
            @('navigationPaneMiddle', 40, [int]($rect.height / 2)),
            @('pageCorner', ($rect.width - 30), ($rect.height - 30))
        )
        $facts.captures["$Label-$($page.ToLower())"] = [ordered]@{
            file = "screenshots/$(Split-Path -Leaf $file)"; window = $rect; inFront = $front; pageSelected = $selected
            meanBrightness = $image.meanBrightness; pixels = $image.pixels
        }
    }
    if ($Pages.Count -gt 1) { [void](Select-Page $hwnd 'Home') }
}

function Get-Brightness {
    $file = Join-Path ([IO.Path]::GetTempPath()) "td-evidence-probe-$PID.png"
    [void](Enter-Window $hwnd)
    Save-Window $file $hwnd
    (Get-ImageFacts $file @()).meanBrightness
}

# Whether the running window showed a settings change. It gets 15 s; then the broadcast is sent
# once more, to tell a missed message from a change the app does not follow. When neither works
# the window is saved as it is and the app restarts, so the look still gets its screenshots.
function Wait-Followed([string]$Label, [scriptblock]$Seen, [string]$Area) {
    if (Wait-Until $Seen 10 500) { return 'yes' }
    if ($Area) {
        [void][TouchDeckEvidence.Native]::BroadcastSettingChange($Area)
        if (Wait-Until $Seen 8 500) { return 'only after a second broadcast' }
    }
    [void](Enter-Window $hwnd)
    Save-Window (Join-Path $shots "$Label-not-followed.png") $hwnd
    Restart-Look
    'no: restarted the app'
}

function Get-NavItemHeight {
    $tab = Find-UiaByName (Get-UiaElement $hwnd) 'Home' 'TabItem'
    if ($tab) { [int]$tab.Current.BoundingRectangle.Height } else { 0 }
}

# Light or dark for apps and the system: the two values Settings > Personalisation > Colours
# writes, then the broadcast it sends.
function Set-ColorMode([bool]$Light) {
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize'
    Set-ItemProperty -LiteralPath $key -Name AppsUseLightTheme -Value ([int]$Light) -Type DWord
    Set-ItemProperty -LiteralPath $key -Name SystemUsesLightTheme -Value ([int]$Light) -Type DWord
    [void][TouchDeckEvidence.Native]::BroadcastSettingChange('ImmersiveColorSet')
}

# Settings > Accessibility > Text size: the value it writes, then a settings broadcast.
function Set-TextScale([int]$Percent) {
    $key = 'HKCU:\Software\Microsoft\Accessibility'
    if (-not (Test-Path -LiteralPath $key)) { New-Item -Path $key | Out-Null }
    Set-ItemProperty -LiteralPath $key -Name TextScaleFactor -Value $Percent -Type DWord
    [void][TouchDeckEvidence.Native]::BroadcastSettingChange('Accessibility')
}

function Get-SystemColors {
    $names = [ordered]@{ window = 5; windowText = 8; highlight = 13; highlightText = 14; hotlight = 26; grayText = 17; buttonFace = 15; buttonText = 18 }
    $colors = [ordered]@{}
    foreach ($name in $names.Keys) {
        $value = [TouchDeckEvidence.Native]::GetSysColor($names[$name])
        $colors[$name] = '#{0:x2}{1:x2}{2:x2}' -f ($value -band 0xff), (($value -shr 8) -band 0xff), (($value -shr 16) -band 0xff)
    }
    $colors
}

function Test-ContrastOn {
    $flags = [uint32]0
    $scheme = $null
    [void][TouchDeckEvidence.Native]::HighContrast([ref]$flags, [ref]$scheme)
    ($flags -band 1) -ne 0
}

# Turns a contrast theme on or off with SPI_SETHIGHCONTRAST, as the Left Alt + Left Shift +
# Print Screen shortcut does. Windows then applies the theme's colours. When they do not change,
# opens the theme file instead, as a double click in Explorer would.
function Set-Contrast([bool]$On) {
    $flags = [uint32]0
    $scheme = $null
    [void][TouchDeckEvidence.Native]::HighContrast([ref]$flags, [ref]$scheme)
    $before = (Get-SystemColors).window
    $flags = [uint32]$(if ($On) { $flags -bor 1 } else { $flags -band 0x7ffffffe })
    if (-not $scheme) { $scheme = 'High Contrast Black' }
    $set = [TouchDeckEvidence.Native]::SetHighContrast($flags, $scheme)
    $method = 'SystemParametersInfo(SPI_SETHIGHCONTRAST)'
    $changed = Wait-Until { (Get-SystemColors).window -ne $before } 20
    if (-not $changed) {
        $theme = if ($On) { "$env:WINDIR\Resources\Ease of Access Themes\hcblack.theme" } else { "$env:WINDIR\Resources\Themes\aero.theme" }
        Start-Process -FilePath $theme
        $method = "opened $(Split-Path -Leaf $theme)"
        $changed = Wait-Until { (Get-SystemColors).window -ne $before } 30
        # Opening a theme file also opens Settings; close it again.
        Start-Sleep -Seconds 2
        Get-Process -Name SystemSettings -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    }
    Start-Sleep -Seconds 4
    $theme = Split-Path -Leaf ([string](Get-ItemProperty -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes' -Name CurrentTheme -ErrorAction SilentlyContinue).CurrentTheme)
    [ordered]@{ method = $method; spiAccepted = $set; scheme = $scheme; theme = $theme; on = (Test-ContrastOn); colorsChanged = [bool]$changed; systemColors = (Get-SystemColors) }
}

try {
    Hide-OtherWindows
    Set-ColorMode $true
    Start-Sleep -Seconds 2
    Start-Look

    # ---- light, and the accessibility tree ------------------------------------------------------
    Save-Pages 'light'
    foreach ($page in 'Home', 'About') {
        [void](Enter-Window $hwnd)
        [void](Select-Page $hwnd $page)
        $nodes = Get-UiaTree (Get-UiaElement $hwnd)
        $name = $page.ToLower()
        Write-Utf8 (Join-Path $uia "$name.txt") (Format-UiaTree $nodes)
        Write-Json (Join-Path $uia "$name.json") $nodes
        Add-Check "uia-$name" "Every focusable control on $page has an accessible name" {
            $unnamed = Get-UnnamedFocusable $nodes
            $focusable = @($nodes | Where-Object { $_.focusable })
            @{
                pass   = $unnamed.Count -eq 0 -and $focusable.Count -gt 0
                detail = "$($nodes.Count) elements, $($focusable.Count) focusable, $($unnamed.Count) without a name" +
                    $(if ($unnamed.Count) { ': ' + (($unnamed | ForEach-Object { "$($_.type) @ $($_.bounds)" }) -join '; ') } else { '' })
            }
        }
    }
    [void](Select-Page $hwnd 'Home')
    $lightBrightness = Get-Brightness

    # What DWM and the app say about Mica, beside what the screenshots show.
    $backdrop = 0
    $read = [TouchDeckEvidence.Native]::BackdropType($hwnd, [ref]$backdrop)
    $said = @(Get-Content -LiteralPath (Join-Path $Out 'logs/look-stderr-1.txt') -ErrorAction SilentlyContinue | Where-Object { $_ -like '*(mock): Mica:*' }) | Select-Object -First 1
    $facts.mica = "td-app reported: $(if ($said) { ($said -replace '^.*\(mock\): Mica: ', '') } else { 'nothing' }). " +
        "DWM reports DWMWA_SYSTEMBACKDROP_TYPE = $backdrop (2 is Mica; HRESULT $('0x{0:x8}' -f $read)). " +
        "Navigation pane pixel in light: $($facts.captures['light-home'].pixels.navigationPaneLow)."
    # #f3f3f3 is the flat colour DWM draws in place of light Mica when it cannot blend the wallpaper.
    if ($backdrop -eq 2 -and $facts.captures['light-home'].pixels.navigationPaneLow -eq '#f3f3f3') {
        $facts.mica += ' DWM accepted Mica and drew its flat fallback colour, with no wallpaper tint: this machine has no GPU, so it cannot show the Mica look.'
    }

    # ---- dark -----------------------------------------------------------------------------------
    Set-ColorMode $false
    $facts.changes.dark = [ordered]@{
        method       = 'AppsUseLightTheme and SystemUsesLightTheme = 0, WM_SETTINGCHANGE "ImmersiveColorSet"'
        followedLive = Wait-Followed 'dark' { (Get-Brightness) -lt ($lightBrightness - 60) } 'ImmersiveColorSet'
    }
    Save-Pages 'dark'
    Set-ColorMode $true
    $facts.changes.light = [ordered]@{
        method       = 'the same values = 1'
        followedLive = Wait-Followed 'light' { (Get-Brightness) -gt ($lightBrightness - 30) } 'ImmersiveColorSet'
    }

    # ---- a contrast theme -----------------------------------------------------------------------
    $contrast = Set-Contrast $true
    $contrast.followedLive = Wait-Followed 'contrast' { (Get-Brightness) -lt ($lightBrightness - 60) } $null
    $facts.changes.contrastOn = $contrast
    Save-Pages 'contrast'
    $contrast = Set-Contrast $false
    Set-ColorMode $true
    $contrast.followedLive = Wait-Followed 'contrast-off' { (Get-Brightness) -gt ($lightBrightness - 30) } $null
    $facts.changes.contrastOff = $contrast

    # ---- text size 200% -------------------------------------------------------------------------
    $height = Get-NavItemHeight
    Set-TextScale 200
    $facts.changes.textScale200 = [ordered]@{
        method       = 'HKCU\Software\Microsoft\Accessibility\TextScaleFactor = 200, WM_SETTINGCHANGE "Accessibility"'
        followedLive = Wait-Followed 'text-200' { (Get-NavItemHeight) -gt $height } 'Accessibility'
        navigationItemHeightBefore = $height; navigationItemHeightAfter = (Get-NavItemHeight)
    }
    Save-Pages 'text-200'

    # ---- the minimum width ----------------------------------------------------------------------
    # Drags the right border as far left as it goes, with the mouse. Windows stops at the
    # minimum the app gives it. Returns the client width that is left.
    function Set-NarrowWindow {
        [void](Enter-Window $hwnd)
        $rect = Get-Rect $hwnd
        $y = $rect.y + [int]($rect.height / 2)
        [void][TouchDeckEvidence.Native]::Drag($rect.x + $rect.width + 2, $y, $rect.x + 40, $y)
        Start-Sleep -Seconds 1
        $client = New-Object TouchDeckEvidence.Native+RECT
        [void][TouchDeckEvidence.Native]::GetClientRect($hwnd, [ref]$client)
        $client.Right - $client.Left
    }
    $narrow200 = Set-NarrowWindow
    Save-Pages 'min-width-text-200' @('Home')
    Save-Pages 'min-width-text-200' @('About')

    Set-TextScale 100
    $facts.changes.textScale100 = [ordered]@{
        method       = 'the same value = 100'
        followedLive = Wait-Followed 'text-100' { (Get-NavItemHeight) -le $height } 'Accessibility'
    }
    $narrow = Set-NarrowWindow
    Save-Pages 'min-width' @('Home')
    $dpi = [TouchDeckEvidence.Native]::GetDpiForSystem()
    $facts.minimumWidth = [ordered]@{ clientWidthPixels = $narrow; clientWidthAtText200 = $narrow200; systemDpi = $dpi; expectedEffectivePixels = 500 }
    Add-Check 'min-width' 'The window stops shrinking at about 500 effective pixels' {
        $effective = $narrow * 96 / $dpi
        @{ pass = $effective -ge 495 -and $effective -le 505; detail = "client width $narrow px at $dpi DPI" }
    }

    Add-Check 'screenshots' 'Home and About were captured in light, dark, a contrast theme and 200% text, and Home at the minimum width' {
        $wanted = 'light-home', 'light-about', 'dark-home', 'dark-about', 'contrast-home', 'contrast-about', 'text-200-home', 'text-200-about', 'min-width-home'
        $missing = @($wanted | Where-Object { -not (Test-Path (Join-Path $shots "$_.png")) })
        $behind = @($facts.captures.Keys | Where-Object { -not $facts.captures[$_].inFront })
        @{
            pass   = $missing.Count -eq 0 -and $behind.Count -eq 0
            detail = "$($facts.captures.Count) screenshots" + $(if ($missing.Count) { "; missing: $($missing -join ', ')" } else { '' }) +
                $(if ($behind.Count) { "; window not in front for: $($behind -join ', ')" } else { '' })
        }
    }
} finally {
    Stop-App $app
    $facts.checks = Get-Checks
    Write-Json (Join-Path $Out 'look.json') $facts
}
