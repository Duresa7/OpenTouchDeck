<#
.SYNOPSIS
Collects all the Windows evidence for Open TouchDeck 2.0's M1 into one folder: runs each script
beside this one against a built td-app, then writes the summary. Fails when any check failed.

.DESCRIPTION
The exe must be a release build with the mock feature:

  cd app
  cargo build --release --locked --package td-app --features mock

Each part runs in a PowerShell process of its own, so one that stops early does not take the
others with it. The parts change the signed-in user's theme, contrast, text size and
Start at sign-in values, restart Explorer, and put nothing back. Run this on a machine you can
throw away, such as a GitHub-hosted runner, from a desktop session.

.EXAMPLE
powershell -ExecutionPolicy Bypass -File .github/scripts/windows-evidence/run.ps1 -Exe app/target/release/td-app.exe -Out evidence
#>
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Exe, [Parameter(Mandatory)][string]$Out)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Exe = (Resolve-Path -LiteralPath $Exe).Path
$Out = (New-Item -ItemType Directory -Force -Path $Out).FullName
$logs = (New-Item -ItemType Directory -Force -Path (Join-Path $Out 'logs')).FullName

# The tray part goes last: it restarts Explorer.
$parts = [ordered]@{}
foreach ($part in 'environment', 'look', 'memory', 'tray') {
    Write-Host "::group::$part"
    $clock = [Diagnostics.Stopwatch]::StartNew()
    # A part's stderr is part of its log, not a reason to stop here.
    $ErrorActionPreference = 'Continue'
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "$part.ps1") -Exe $Exe -Out $Out 2>&1 |
        ForEach-Object { "$_" } | Tee-Object -FilePath (Join-Path $logs "$part.log")
    $code = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    $parts[$part] = [ordered]@{ exitCode = $code; seconds = [Math]::Round($clock.Elapsed.TotalSeconds) }
    Write-Host "::endgroup::"
    Write-Host "$part finished with exit code $code in $($parts[$part].seconds) s"
}
[IO.File]::WriteAllText((Join-Path $Out 'parts.json'), ($parts | ConvertTo-Json), (New-Object Text.UTF8Encoding $false))

& (Join-Path $PSScriptRoot 'summary.ps1') -Out $Out
