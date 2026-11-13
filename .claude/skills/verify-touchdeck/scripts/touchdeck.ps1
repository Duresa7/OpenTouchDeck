# Runs ESP-IDF tools for the TouchDeck repo without a pre-activated shell.
#
#   pwsh -NoProfile -File touchdeck.ps1 idf <idf.py args...>      e.g. idf build, idf flash
#   pwsh -NoProfile -File touchdeck.ps1 idf build-crashing        build firmware\ with -DTD_TEST_BOOT_CRASH=1 into build-crash\
#   pwsh -NoProfile -File touchdeck.ps1 idf build-newer [--version <v>] [--out-dir <abs>]
#                                                                  test only: build firmware\ with a higher version into
#                                                                  build-newer\ and pack it as a .tdfw; prints its JSON
#   pwsh -NoProfile -File touchdeck.ps1 serial <command> [opts]   touchdeck_serial.py (ports, doctor, capture, cmd)
#   pwsh -NoProfile -File touchdeck.ps1 unit [--port <port>]      build + flash firmware\test_app, run its Unity tests
#   pwsh -NoProfile -File touchdeck.ps1 esptool <args...>         python -m esptool, e.g. esptool chip-id
#
# Finds the selected ESP-IDF install from the ESP-IDF Installation Manager's
# eim_idf.json (override the path with $env:TOUCHDECK_EIM_JSON).
#
# Serial port: an explicit -p/--port wins, else $env:TOUCHDECK_PORT, else the tools find
# the board's single USB-Serial/JTAG port themselves.
#
# Works on the checkout in $env:TOUCHDECK_ROOT, else the checkout this script lives in.
# The ESP-IDF project is <root>\firmware when that exists, else <root> (pre-M1-01 layout).
#
# No param() block on purpose: an advanced script would claim --out and other
# options as its own common parameters. Everything after the tool name passes through.
$ErrorActionPreference = 'Stop'
$Tool = $args[0]
$ToolArgs = @($args | Select-Object -Skip 1)
if ($Tool -notin 'idf', 'serial', 'esptool', 'unit', 'link') { throw "usage: touchdeck.ps1 idf|serial|esptool|unit|link <args...>" }

$eimJson = if ($env:TOUCHDECK_EIM_JSON) { $env:TOUCHDECK_EIM_JSON } else { 'C:\Espressif\tools\eim_idf.json' }
if (-not (Test-Path $eimJson)) { throw "ESP-IDF install list not found at $eimJson. Install ESP-IDF with EIM or set TOUCHDECK_EIM_JSON." }
$eim = Get-Content $eimJson -Raw | ConvertFrom-Json
$install = $eim.idfInstalled | Where-Object id -eq $eim.idfSelectedId | Select-Object -First 1
if (-not $install) { throw "No selected ESP-IDF install in $eimJson." }

# Launched from Git Bash, MSYSTEM is inherited and idf.py refuses to run, printing
# "MSys/Mingw is no longer supported" and exiting 0 without building or flashing.
Remove-Item Env:MSYSTEM -ErrorAction SilentlyContinue

# The activation script prints a banner; keep stdout clean for JSON consumers.
. $install.activationScript *> $null

# The board's port, unless the caller passed one.
$portArgs = @()
if ($env:TOUCHDECK_PORT -and -not ($ToolArgs | Where-Object { $_ -in '-p', '--port' -or $_ -like '--port=*' })) {
    $portArgs = if ($Tool -eq 'idf') { @('-p', $env:TOUCHDECK_PORT) } else { @('--port', $env:TOUCHDECK_PORT) }
}

$repoRoot = if ($env:TOUCHDECK_ROOT) { (Resolve-Path $env:TOUCHDECK_ROOT).Path } else { (Resolve-Path (Join-Path $PSScriptRoot '..\..\..\..')).Path }
$project = if (Test-Path (Join-Path $repoRoot 'firmware\CMakeLists.txt')) { Join-Path $repoRoot 'firmware' } else { $repoRoot }
$env:TOUCHDECK_PROJECT = $project
Set-Location $repoRoot

if ($Tool -eq 'idf' -and $ToolArgs.Count -ge 1 -and $ToolArgs[0] -eq 'build-crashing') {
    # An image that aborts two seconds after app_main, in build-crash\ so build\ stays the good one. It is for the
    # rollback check: update the deck to it and the bootloader must put the previous slot back.
    & $install.python (Join-Path $env:IDF_PATH 'tools\idf.py') -C $project -B (Join-Path $project 'build-crash') -DTD_TEST_BOOT_CRASH=1 build @($ToolArgs | Select-Object -Skip 1)
} elseif ($Tool -eq 'idf' -and $ToolArgs.Count -ge 1 -and $ToolArgs[0] -eq 'build-newer') {
    # Test only. The same source with PROJECT_VER raised one patch above `git describe`, so the image's elf_sha256
    # differs from build\ and a Companion bundling it offers an update to a Device running build\.
    $rest = @($ToolArgs | Select-Object -Skip 1)
    $versionAt = [array]::IndexOf($rest, '--version')
    $outAt = [array]::IndexOf($rest, '--out-dir')
    $buildDir = Join-Path $project 'build-newer'
    $outDir = if ($outAt -ge 0) { $rest[$outAt + 1] } else { $buildDir }
    if ($versionAt -ge 0) {
        $version = $rest[$versionAt + 1]
    } else {
        $describe = (& git -C $repoRoot describe --tags --always --dirty).Trim()
        if ($describe -notmatch '^v(\d+)\.(\d+)\.(\d+)(.*)$') { throw "git describe gave '$describe'; pass --version." }
        $version = "v$($Matches[1]).$($Matches[2]).$([int]$Matches[3] + 1)-newer$($Matches[4])"
    }
    if (-not $version -or $version.Length -gt 31 -or $version -notmatch '^[A-Za-z0-9._+-]+$') { throw "Version '$version' must be 1 to 31 characters of A-Z a-z 0-9 . _ + -." }
    & $install.python (Join-Path $env:IDF_PATH 'tools\idf.py') -C $project -B $buildDir "-DPROJECT_VER=$version" build | ForEach-Object { [Console]::Error.WriteLine($_) }
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & $install.python (Join-Path $project 'tools\make_bundle.py') --build-dir $buildDir --out-dir $outDir
} elseif ($Tool -eq 'idf') {
    # A caller-supplied -C (e.g. firmware\test_app) wins over the main project.
    $projectArgs = if ($ToolArgs -contains '-C') { @() } else { @('-C', $project) }
    & $install.python (Join-Path $env:IDF_PATH 'tools\idf.py') @projectArgs @portArgs @ToolArgs
} elseif ($Tool -eq 'unit') {
    $testProject = Join-Path $repoRoot 'firmware\test_app'
    if (-not (Test-Path (Join-Path $testProject 'CMakeLists.txt'))) { throw "No unit test app at $testProject." }
    $idfPy = Join-Path $env:IDF_PATH 'tools\idf.py'
    $portIndex = [array]::IndexOf($ToolArgs, '--port')
    $flashPort = if ($portIndex -ge 0 -and $portIndex + 1 -lt $ToolArgs.Count) { @('-p', $ToolArgs[$portIndex + 1]) } elseif ($env:TOUCHDECK_PORT) { @('-p', $env:TOUCHDECK_PORT) } else { @() }
    # Build and flash chatter goes to stderr, so stdout holds only the JSON report.
    & $install.python $idfPy -C $testProject build | ForEach-Object { [Console]::Error.WriteLine($_) }
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & $install.python $idfPy -C $testProject @flashPort flash | ForEach-Object { [Console]::Error.WriteLine($_) }
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & $install.python (Join-Path $PSScriptRoot 'touchdeck_serial.py') unit @ToolArgs
} elseif ($Tool -eq 'link') {
    & $install.python (Join-Path $PSScriptRoot 'touchdeck_link.py') @ToolArgs
} elseif ($Tool -eq 'esptool') {
    & $install.python -m esptool @portArgs @ToolArgs
} else {
    & $install.python (Join-Path $PSScriptRoot 'touchdeck_serial.py') @ToolArgs
}
exit $LASTEXITCODE
