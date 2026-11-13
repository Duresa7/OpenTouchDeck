<#
.SYNOPSIS
Measures td-app's memory with one mock Device: with the window open on Home, then hidden to the
tray. Writes memory.json.

.DESCRIPTION
Method: start the release build with `--mock-devices 1`, wait for the window, let it settle for
30 s, then read the process counters five times, 2 s apart, and report the median.

  workingSetBytes         PROCESS_MEMORY_COUNTERS_EX.WorkingSetSize (Process.WorkingSet64)
  privateBytes            PROCESS_MEMORY_COUNTERS_EX.PrivateUsage (Process.PrivateMemorySize64)
  privateWorkingSetBytes  the "Working Set - Private" performance counter, which is the figure
                          Task Manager shows as Memory

.EXAMPLE
powershell -File .github/scripts/windows-evidence/memory.ps1 -Exe app/target/release/td-app.exe -Out evidence
#>
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Exe, [Parameter(Mandatory)][string]$Out, [int]$SettleSeconds = 30)
. "$PSScriptRoot/lib.ps1"

$Exe = (Resolve-Path -LiteralPath $Exe).Path
$Out = (New-Item -ItemType Directory -Force -Path $Out).FullName
$data = Join-Path ([IO.Path]::GetTempPath()) "td-evidence-memory-$PID"

function Get-Median([long[]]$Values) {
    $sorted = @($Values | Sort-Object)
    $sorted[[int][Math]::Floor($sorted.Count / 2)]
}

function Measure-App($Process) {
    $samples = @(for ($i = 0; $i -lt 5; $i++) {
            if ($i) { Start-Sleep -Seconds 2 }
            $Process.Refresh()
            $private = $null
            try {
                $counter = Get-Counter -Counter "\Process($($Process.ProcessName))\Working Set - Private" -ErrorAction Stop
                $private = [long]$counter.CounterSamples[0].CookedValue
            } catch { }
            [ordered]@{ workingSetBytes = $Process.WorkingSet64; privateBytes = $Process.PrivateMemorySize64; privateWorkingSetBytes = $private }
        })
    $result = [ordered]@{}
    foreach ($name in 'workingSetBytes', 'privateBytes', 'privateWorkingSetBytes') {
        $values = @($samples | ForEach-Object { $_[$name] } | Where-Object { $null -ne $_ })
        if ($values.Count) {
            $result[$name] = Get-Median $values
            $result[$name -replace 'Bytes$', 'MiB'] = [Math]::Round((Get-Median $values) / 1MB, 1)
        }
    }
    $result.peakWorkingSetBytes = $Process.PeakWorkingSet64
    $result.threads = $Process.Threads.Count
    $result.handles = $Process.HandleCount
    $result.samples = $samples
    $result
}

$app = $null
$facts = [ordered]@{
    method      = "td-app release build with the mock feature, started with --mock-devices 1. After the window appears it settles for $SettleSeconds s; each figure is the median of 5 readings 2 s apart. workingSetBytes is PROCESS_MEMORY_COUNTERS_EX.WorkingSetSize, privateBytes is PrivateUsage, and privateWorkingSetBytes is the 'Working Set - Private' performance counter (Task Manager's Memory column)."
    executable  = Split-Path -Leaf $Exe
    arguments   = '--mock-devices 1'
    mockDevices = 1
}
try {
    Hide-OtherWindows
    $app = Start-App $Exe @('--mock-devices', '1') $data (Join-Path $Out 'logs/memory-stderr.txt')
    $hwnd = Wait-AppWindow $app
    [void](Enter-Window $hwnd)
    Start-Sleep -Seconds $SettleSeconds
    $facts.windowOpenOnHome = Measure-App $app
    # Which graphics libraries the process loaded tells which renderer drew the window.
    $app.Refresh()
    $facts.graphicsModules = @($app.Modules | ForEach-Object { $_.ModuleName.ToLower() } |
            Where-Object { $_ -match '^(opengl32|libegl|libglesv2|d3d\d+|d3d10warp|dxgi|dcomp|vulkan-1|dwrite|d2d1)\.dll$' } | Sort-Object -Unique)

    $hidden = Hide-ToTray $hwnd $null
    if ($hidden.hidden) {
        Start-Sleep -Seconds 15
        $facts.windowHiddenToTray = Measure-App $app
    }
    Add-Check 'memory' 'Working set and private bytes were measured with one mock Device' {
        $open = $facts.windowOpenOnHome
        @{
            pass   = $open.workingSetBytes -gt 0 -and $open.privateBytes -gt 0
            detail = "window open: working set $($open.workingSetMiB) MiB, private bytes $($open.privateMiB) MiB" +
                $(if ($facts.Contains('windowHiddenToTray')) { "; hidden to tray: working set $($facts.windowHiddenToTray.workingSetMiB) MiB, private bytes $($facts.windowHiddenToTray.privateMiB) MiB" } else { '' })
        }
    }
} finally {
    Stop-App $app
    $facts.checks = Get-Checks
    Write-Json (Join-Path $Out 'memory.json') $facts
}
