<#
.SYNOPSIS
Records what this Windows machine is and what its desktop can show, so the screenshots and
measurements beside it can be read correctly. Writes environment.json, a screenshot of the
desktop and the taskbar's UI Automation tree.

.DESCRIPTION
GitHub's windows-latest runner is Windows Server, not Windows 11. This script records the facts
that decide what the evidence can prove there: the OS build, whether a desktop session with
Explorer exists, the display adapter, and whether Segoe UI Variable, transparency and the
contrast themes are present.

.EXAMPLE
powershell -File .github/scripts/windows-evidence/environment.ps1 -Out evidence
#>
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Out)
. "$PSScriptRoot/lib.ps1"

$Out = (New-Item -ItemType Directory -Force -Path $Out).FullName

function Get-Reg([string]$Key, [string]$Name) {
    try { (Get-ItemProperty -LiteralPath $Key -Name $Name -ErrorAction Stop).$Name } catch { $null }
}

$nt = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
$os = Get-CimInstance Win32_OperatingSystem
$personalize = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize'

$accent = $null
$textScale = $null
try {
    $settings = [Windows.UI.ViewManagement.UISettings, Windows.UI.ViewManagement, ContentType = WindowsRuntime]::new()
    $color = $settings.GetColorValue([Windows.UI.ViewManagement.UIColorType, Windows.UI.ViewManagement, ContentType = WindowsRuntime]::Accent)
    $accent = '#{0:x2}{1:x2}{2:x2}' -f $color.R, $color.G, $color.B
    $textScale = $settings.TextScaleFactor
} catch { Write-Host "UISettings is not readable here: $($_.Exception.Message)" }

$contrastFlags = [uint32]0
$contrastScheme = $null
[void][TouchDeckEvidence.Native]::HighContrast([ref]$contrastFlags, [ref]$contrastScheme)

$fonts = New-Object Drawing.Text.InstalledFontCollection
$segoe = @($fonts.Families | ForEach-Object { $_.Name } | Where-Object { $_ -like 'Segoe UI*' })

$explorer = @(Get-Process -Name explorer -ErrorAction SilentlyContinue)
$taskbar = [IntPtr]::Zero
foreach ($hwnd in [TouchDeckEvidence.Native]::TopLevelWindows(0)) {
    if ([TouchDeckEvidence.Native]::ClassName($hwnd) -eq 'Shell_TrayWnd') { $taskbar = $hwnd }
}
$cursor = New-Object TouchDeckEvidence.Native+POINT
$cursorRead = [TouchDeckEvidence.Native]::GetCursorPos([ref]$cursor)
$screen = [Windows.Forms.Screen]::PrimaryScreen

$facts = [ordered]@{
    os       = [ordered]@{
        name             = $os.Caption
        version          = $os.Version
        build            = "$(Get-Reg $nt 'CurrentBuildNumber').$(Get-Reg $nt 'UBR')"
        displayVersion   = Get-Reg $nt 'DisplayVersion'
        edition          = Get-Reg $nt 'EditionID'
        installationType = Get-Reg $nt 'InstallationType'
        # 1 is a workstation (Windows 11); 3 is a server.
        productType      = $os.ProductType
        runnerImage      = "$env:ImageOS $env:ImageVersion".Trim()
    }
    session  = [ordered]@{
        id              = (Get-Process -Id $PID).SessionId
        name            = $env:SESSIONNAME
        interactive     = [Environment]::UserInteractive
        explorerRunning = $explorer.Count -gt 0
        explorerSessions = @($explorer | ForEach-Object { $_.SessionId } | Sort-Object -Unique)
        taskbarFound    = $taskbar -ne [IntPtr]::Zero
        taskbar         = $(if ($taskbar -ne [IntPtr]::Zero) { Get-Rect $taskbar } else { $null })
        cursorReadable  = $cursorRead
    }
    display  = [ordered]@{
        width          = $screen.Bounds.Width
        height         = $screen.Bounds.Height
        bitsPerPixel   = $screen.BitsPerPixel
        systemDpi      = [TouchDeckEvidence.Native]::GetDpiForSystem()
        # DWM draws Mica; it runs on the adapter below (a software one on a machine with no GPU).
        dwmComposition = [TouchDeckEvidence.Native]::Composition()
        adapters       = @(Get-CimInstance Win32_VideoController | ForEach-Object { "$($_.Name) (driver $($_.DriverVersion))" })
    }
    look     = [ordered]@{
        appsUseLightTheme   = Get-Reg $personalize 'AppsUseLightTheme'
        systemUsesLightTheme = Get-Reg $personalize 'SystemUsesLightTheme'
        # Mica needs transparency effects on; 0 here makes DWM draw a solid colour instead.
        enableTransparency  = Get-Reg $personalize 'EnableTransparency'
        accent              = $accent
        textScaleFactor     = $textScale
        textScaleRegistry   = Get-Reg 'HKCU:\Software\Microsoft\Accessibility' 'TextScaleFactor'
        highContrastOn      = ($contrastFlags -band 1) -ne 0
        highContrastFlags   = '0x{0:x}' -f $contrastFlags
        highContrastScheme  = $contrastScheme
        contrastThemeFiles  = @(Get-ChildItem "$env:WINDIR\Resources\Ease of Access Themes" -Filter *.theme -ErrorAction SilentlyContinue | ForEach-Object { $_.Name })
        wallpaper           = Get-Reg 'HKCU:\Control Panel\Desktop' 'WallPaper'
    }
    fonts    = [ordered]@{
        segoeFamilies        = $segoe
        segoeUiVariableText  = $segoe -contains 'Segoe UI Variable Text'
        segoeUiVariableDisplay = $segoe -contains 'Segoe UI Variable Display'
        segoeUiVariableFile  = Test-Path "$env:WINDIR\Fonts\SegUIVar.ttf"
    }
}
Write-Json (Join-Path $Out 'environment.json') $facts
$facts | ConvertTo-Json -Depth 6 | Write-Host

Save-Screen (Join-Path $Out 'desktop-before.png')
if ($taskbar -ne [IntPtr]::Zero) {
    Write-Utf8 (Join-Path $Out 'taskbar-uia-before.txt') (Format-UiaTree (Get-UiaTree (Get-UiaElement $taskbar)))
}
