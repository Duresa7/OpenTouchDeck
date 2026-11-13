<#
.SYNOPSIS
Writes the run summary from the evidence folder: the runner's OS and build, how memory was
measured, every check's result, and what this machine can and cannot show. Saves it as
summary.md and results.json, adds it to the GitHub job summary, and fails when a check failed.

.EXAMPLE
powershell -File .github/scripts/windows-evidence/summary.ps1 -Out evidence
#>
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Out)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Read-Json([string]$Name) {
    $path = Join-Path $Out $Name
    if (Test-Path -LiteralPath $path) { Get-Content -LiteralPath $path -Raw | ConvertFrom-Json } else { $null }
}
function Get-Field($Object, [string]$Name) {
    if ($null -ne $Object -and $Object.PSObject.Properties[$Name]) { $Object.$Name } else { $null }
}
function Format-Cell([string]$Text) { ($Text -replace '\|', '\|') -replace '\r?\n', ' ' }
function Format-Bool($Value) { if ($null -eq $Value) { 'not recorded' } elseif ($Value) { 'yes' } else { 'no' } }

$environment = Read-Json 'environment.json'
$look = Read-Json 'look.json'
$memory = Read-Json 'memory.json'
$tray = Read-Json 'tray.json'
$parts = Read-Json 'parts.json'

$checks = New-Object System.Collections.Generic.List[object]
foreach ($part in @('look', $look), @('memory', $memory), @('tray', $tray)) {
    $found = @(Get-Field $part[1] 'checks')
    foreach ($check in $found) { if ($null -ne $check) { $checks.Add($check) } }
}
# A part that stopped early fails the run, whatever its checks said before it stopped.
foreach ($name in 'environment', 'look', 'memory', 'tray') {
    $code = Get-Field (Get-Field $parts $name) 'exitCode'
    if ($null -ne $parts -and $code -ne 0) {
        $checks.Add([pscustomobject]@{ id = "part-$name"; name = "$name.ps1 ran to the end"; pass = $false; detail = "exit code $code; see logs/$name.log" })
    }
}
$expected = 'uia-home', 'uia-about', 'min-width', 'screenshots', 'memory', 'tray-1', 'tray-2', 'tray-3', 'tray-4', 'tray-5', 'tray-6', 'tray-7', 'tray-8'
foreach ($id in $expected) {
    if (-not @($checks | Where-Object { $_.id -eq $id }).Count) {
        $checks.Add([pscustomobject]@{ id = $id; name = 'did not run'; pass = $false; detail = 'the script stopped before this check' })
    }
}
$order = @{}
for ($i = 0; $i -lt $expected.Count; $i++) { $order[$expected[$i]] = $i }
$checks = @($checks | Sort-Object { if ($order.ContainsKey($_.id)) { $order[$_.id] } else { 99 } })
$failed = @($checks | Where-Object { -not $_.pass })

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add('# Windows evidence: Open TouchDeck 2.0 M1')
$lines.Add('')
if ($environment) {
    $os = $environment.os
    $lines.Add("**Runner:** $($os.name), version $($os.displayVersion), build $($os.build) ($($os.installationType)), image $($os.runnerImage).")
    if ($os.productType -ne 1) {
        $lines.Add('')
        $lines.Add('This is Windows Server, not Windows 11. It shares the Windows 11 24H2 code base, but it is not the system users run. Read the screenshots as Server with the Windows 11 shell.')
    }
}
$lines.Add('')
$lines.Add("**Result:** $($checks.Count - $failed.Count) of $($checks.Count) checks passed.")
$lines.Add('')
$lines.Add('| Check | Result | What was seen |')
$lines.Add('| --- | --- | --- |')
foreach ($check in $checks) {
    $result = if ($check.pass) { 'pass' } else { '**FAIL**' }
    $lines.Add("| $($check.id): $(Format-Cell $check.name) | $result | $(Format-Cell $check.detail) |")
}

if ($memory) {
    $lines.Add('')
    $lines.Add('## Memory')
    $lines.Add('')
    $lines.Add("Method: $($memory.method)")
    $lines.Add('')
    $lines.Add('| State | Working set | Private bytes | Private working set (Task Manager) |')
    $lines.Add('| --- | --- | --- | --- |')
    foreach ($state in @('windowOpenOnHome', 'Window open on Home'), @('windowHiddenToTray', 'Window hidden to the tray')) {
        $row = Get-Field $memory $state[0]
        if ($row) {
            $privateSet = Get-Field $row 'privateWorkingSetMiB'
            $privateSet = if ($privateSet) { "$privateSet MiB" } else { 'not read' }
            $lines.Add("| $($state[1]) | $($row.workingSetMiB) MiB | $($row.privateMiB) MiB | $privateSet |")
        }
    }
    $modules = @(Get-Field $memory 'graphicsModules')
    $lines.Add('')
    $loaded = if ($modules.Count) { $modules -join ', ' } else { 'none recorded' }
    $lines.Add("Graphics libraries loaded by td-app: $loaded.")
}

if ($environment) {
    $display = $environment.display
    $facts = $environment.look
    $fonts = $environment.fonts
    $lines.Add('')
    $lines.Add('## What this machine can show')
    $lines.Add('')
    $lines.Add('| Subject | On this runner |')
    $lines.Add('| --- | --- |')
    $lines.Add("| Desktop | $($display.width)x$($display.height) at $($display.systemDpi) DPI, session $($environment.session.id), Explorer running: $(Format-Bool $environment.session.explorerRunning), taskbar found: $(Format-Bool $environment.session.taskbarFound) |")
    $lines.Add("| Display adapter | $(Format-Cell ($display.adapters -join '; ')). DWM composition: $(Format-Bool $display.dwmComposition) |")
    $transparency = if ($facts.enableTransparency -eq 1) { 'on' } elseif ($null -eq $facts.enableTransparency) { 'not set' } else { 'off' }
    $lines.Add("| Mica | The app asks DWM for Mica from build 22621; this is build $($environment.os.build). Transparency effects: $transparency. $(Format-Cell ([string](Get-Field $look 'mica'))) |")
    $lines.Add("| Accent colour | $($facts.accent) (the Windows default; the runner has no user choice to follow) |")
    $lines.Add("| Segoe UI Variable | Text: $(Format-Bool $fonts.segoeUiVariableText), Display: $(Format-Bool $fonts.segoeUiVariableDisplay), font file present: $(Format-Bool $fonts.segoeUiVariableFile) |")
    $lines.Add("| Contrast themes | $(Format-Cell ($facts.contrastThemeFiles -join ', ')) |")
    $lines.Add("| Text size | $($facts.textScaleFactor) before the run |")
}

if ($look) {
    $lines.Add('')
    $lines.Add('## Settings changed while the app ran')
    $lines.Add('')
    $lines.Add('| Change | How | The window followed without a restart |')
    $lines.Add('| --- | --- | --- |')
    foreach ($change in $look.changes.PSObject.Properties) {
        $lines.Add("| $($change.Name) | $(Format-Cell ([string]$change.Value.method)) | $($change.Value.followedLive) |")
    }
    $contrast = Get-Field $look.changes 'contrastOn'
    if ($contrast) {
        $colors = ($contrast.systemColors.PSObject.Properties | ForEach-Object { "$($_.Name) $($_.Value)" }) -join ', '
        $lines.Add('')
        $lines.Add("Contrast theme: $($contrast.theme), high contrast flag on: $(Format-Bool $contrast.on), system colours: $colors.")
    }
    $width = Get-Field $look 'minimumWidth'
    if ($width) {
        $lines.Add('')
        $lines.Add("Minimum width: the client area stops at $($width.clientWidthPixels) px at $($width.systemDpi) DPI ($($width.clientWidthAtText200) px at 200% text).")
    }
    $lines.Add('')
    $lines.Add("App restarts needed to show a setting: $($look.restarts).")
    $lines.Add('')
    $lines.Add('A change marked "no" was made the way the table says, not through the Settings app. It shows that this method did not reach the running window; it does not show what Settings itself does. The look at startup is in the screenshots either way.')
}

if ($tray) {
    $lines.Add('')
    $lines.Add('## Notification area')
    $lines.Add('')
    $lines.Add("Where Windows put the icon: $(Get-Field $tray 'iconPlace'). After the Explorer restart: $(Get-Field (Get-Field $tray 'explorerRestart') 'iconPlace').")
}

# What td-app itself wrote: its mock-only notes on Mica and settings changes, and any error.
$said = @(Get-ChildItem -Path (Join-Path $Out 'logs') -Filter '*stderr*.txt' -ErrorAction SilentlyContinue | ForEach-Object {
        $file = $_.Name
        Get-Content -LiteralPath $_.FullName | Where-Object { $_ -match '^Open TouchDeck|panicked|^thread ' } | Sort-Object -Unique | ForEach-Object { "- ${file}: $_" }
    })
$lines.Add('')
$lines.Add('## What td-app wrote to stderr')
$lines.Add('')
if ($said.Count) { foreach ($line in $said) { $lines.Add($line) } } else { $lines.Add('Nothing from the app itself.') }

$lines.Add('')
$lines.Add('## Files in the artifact')
$lines.Add('')
$lines.Add('- `screenshots/`: Home and About in each look, the close notice, the notification area, the tray menu and the exe icon.')
$lines.Add('- `uia/`: the UI Automation tree of Home, About, the close notice, the tray menu and the taskbar.')
$lines.Add('- `environment.json`, `look.json`, `memory.json`, `tray.json`, `results.json`: the facts behind this summary.')
$lines.Add('- `logs/`: each script''s output and td-app''s stderr.')

$text = ($lines -join "`n") + "`n"
$utf8 = New-Object Text.UTF8Encoding $false
[IO.File]::WriteAllText((Join-Path $Out 'summary.md'), $text, $utf8)
[IO.File]::WriteAllText((Join-Path $Out 'results.json'), ((
            [ordered]@{ passed = $checks.Count - $failed.Count; failed = $failed.Count; checks = $checks } | ConvertTo-Json -Depth 6) + "`n"), $utf8)
if ($env:GITHUB_STEP_SUMMARY) { [IO.File]::AppendAllText($env:GITHUB_STEP_SUMMARY, $text, $utf8) }
Write-Host $text

if ($failed.Count) {
    Write-Host "::error::$($failed.Count) check(s) failed: $(($failed | ForEach-Object { $_.id }) -join ', ')"
    exit 1
}
