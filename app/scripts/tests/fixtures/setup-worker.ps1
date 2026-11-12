param([string]$SetupScript, [string]$FixtureRoot)
$ErrorActionPreference = 'Stop'
$config = Get-Content "$FixtureRoot/config.json" -Raw | ConvertFrom-Json
$pwsh = (Get-Process -Id $PID).Path
$native = Join-Path $PSScriptRoot 'native-tool.ps1'
$env:TOUCHDECK_SETUP_FIXTURE = $FixtureRoot
# No real tool directory survives in the worker's environment.
$env:PATH = "$FixtureRoot/bin"
$env:USERPROFILE = "$FixtureRoot/home"
$env:APPDATA = "$FixtureRoot/appdata"
$env:ProgramFiles = "$FixtureRoot/programfiles"
${env:ProgramFiles(x86)} = "$FixtureRoot/programfiles-x86"
foreach ($dir in @('bin', 'staged', 'installed')) { New-Item -ItemType Directory "$FixtureRoot/$dir" | Out-Null }
foreach ($tool in @('winget', 'rustup', 'rustc')) {
    $body = "@echo off`r`n`"$pwsh`" -NoProfile -File `"$native`" -Tool $tool %*`r`nexit /b %errorlevel%`r`n"
    Set-Content "$FixtureRoot/staged/$tool.cmd" $body
    if ($tool -notin $config.absent) { Copy-Item "$FixtureRoot/staged/$tool.cmd" "$FixtureRoot/bin/$tool.cmd" }
}
. $SetupScript
# Only OS reads are substituted. Native command lookup, invocation, output and exit
# handling use real child processes; every resolved executable must be a fixture.
function Get-InstalledPaths { if (Test-Path "$FixtureRoot/path-installed") { "$FixtureRoot/installed" } }
function Get-MsvcInstallation { if (!$config.missingMsvc -or (Test-Path "$FixtureRoot/msvc")) { 'fixture-msvc' } }
$findTool = ${function:Find-Tool}
function Find-Tool($name) {
    $command = & $findTool $name
    if ($command -and ![IO.Path]::GetFullPath($command.Source).StartsWith("$FixtureRoot\", [StringComparison]::OrdinalIgnoreCase)) {
        throw "Test refused a non-fixture tool: $($command.Source)"
    }
    $command
}
try {
    # Exercise explicit native checks even with this enabled by a caller.
    $PSNativeCommandUseErrorActionPreference = $true
    Invoke-TouchDeckSetup
    if ($config.repeat) { Invoke-TouchDeckSetup }
    exit 0
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
