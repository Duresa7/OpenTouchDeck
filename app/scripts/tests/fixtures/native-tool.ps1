# Only launched by the isolated test worker's .cmd fixtures. No host tool calls.
param([string]$Tool)
$ErrorActionPreference = 'Stop'
$root = $env:TOUCHDECK_SETUP_FIXTURE
$config = Get-Content "$root/config.json" -Raw | ConvertFrom-Json
$call = @{ tool = $Tool; arguments = @($args) } | ConvertTo-Json -Compress
Add-Content "$root/calls.jsonl" $call
if ($config.failTool -eq $Tool -and (!$config.failArgument -or $config.failArgument -in $args)) {
    [Console]::Error.WriteLine('fixture native command failure')
    exit 17
}
switch ($Tool) {
    'winget' {
        if ($args[0] -ne 'install') { throw 'Unexpected winget fixture command' }
        $id = $args[([array]::IndexOf($args, '--id') + 1)]
        switch ($id) {
            'Rustlang.Rustup' {
                Copy-Item "$root/staged/rustup.cmd" "$root/installed/rustup.cmd"
                Copy-Item "$root/staged/rustc.cmd" "$root/installed/rustc.cmd"
            }
            'Microsoft.VisualStudio.2022.BuildTools' { if (!$config.noInstalledMsvc) { Set-Content "$root/msvc" 'installed' } }
            default { throw "Unexpected installer: $id" }
        }
        Set-Content "$root/path-installed" 'registered PATH'
    }
    'rustup' { }
    'rustc' { 'rustc 1.92.0 (fixture)' }
    default { throw "Unexpected tool: $Tool" }
}
exit 0
