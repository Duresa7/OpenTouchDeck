$ErrorActionPreference = 'Stop'
$root = if ($env:TOUCHDECK_ROOT) { (Resolve-Path $env:TOUCHDECK_ROOT).Path } else { (Resolve-Path "$PSScriptRoot/../../../..").Path }
& node "$PSScriptRoot/tdapp.mjs" $root @args
exit $LASTEXITCODE
