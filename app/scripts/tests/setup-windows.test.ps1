# Dependency-free, process-isolated tests. Never execute the installer on the host.
#   pwsh -NoProfile -File app/scripts/tests/setup-windows.test.ps1
param([string]$SetupScript = (Join-Path $PSScriptRoot '../setup-windows.ps1'))
$ErrorActionPreference = 'Stop'
if (!$IsWindows) { throw 'These .cmd process fixtures require Windows and PowerShell 7.' }
$SetupScript = (Resolve-Path -LiteralPath $SetupScript).Path
$pwsh = (Get-Process -Id $PID).Path
$cases = @(
    @{ name = 'ready tools and repeat'; config = @{ repeat = $true }; ok = $true; noInstall = $true },
    @{ name = 'fresh rust via refreshed PATH'; config = @{ absent = @('rustup', 'rustc') }; ok = $true; installRust = $true },
    @{ name = 'winget install failure'; config = @{ absent = @('rustup', 'rustc'); failTool = 'winget'; failArgument = 'install' }; error = 'winget install.*exit code 17.*rerun setup' },
    @{ name = 'winget missing'; config = @{ absent = @('rustup', 'rustc', 'winget') }; error = 'winget was not found.*App Installer' },
    @{ name = 'rustup default native failure'; config = @{ failTool = 'rustup'; failArgument = 'default' }; error = 'rustup default.*exit code 17.*Repair rustup' },
    @{ name = 'rustup component native failure'; config = @{ failTool = 'rustup'; failArgument = 'component' }; error = 'rustup component.*exit code 17' },
    @{ name = 'rustc native failure'; config = @{ failTool = 'rustc' }; error = 'rustc --version.*exit code 17' },
    @{ name = 'missing MSVC installed'; config = @{ missingMsvc = $true }; ok = $true },
    @{ name = 'MSVC not detected after installer'; config = @{ missingMsvc = $true; noInstalledMsvc = $true }; error = 'MSVC tools still not detected.*Windows SDK' }
)
$failed = 0
foreach ($case in $cases) {
    $root = Join-Path ([IO.Path]::GetTempPath()) "touchdeck-setup-$([guid]::NewGuid())"
    New-Item -ItemType Directory $root | Out-Null
    try {
        $case.config | ConvertTo-Json | Set-Content "$root/config.json"
        $start = [Diagnostics.ProcessStartInfo]::new($pwsh)
        $start.UseShellExecute = $false
        $start.CreateNoWindow = $true
        $start.RedirectStandardOutput = $true
        $start.RedirectStandardError = $true
        foreach ($arg in @('-NoProfile', '-File', "$PSScriptRoot/fixtures/setup-worker.ps1", '-SetupScript', $SetupScript, '-FixtureRoot', $root)) {
            $start.ArgumentList.Add($arg)
        }
        $process = [Diagnostics.Process]::Start($start)
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (!$process.WaitForExit(45000)) { $process.Kill($true); throw 'Worker timed out' }
        $output = $stdoutTask.GetAwaiter().GetResult() + $stderrTask.GetAwaiter().GetResult()
        $calls = if (Test-Path "$root/calls.jsonl") { @(Get-Content "$root/calls.jsonl" | ConvertFrom-Json) } else { @() }
        if ($case.ok) {
            if ($process.ExitCode -ne 0) { throw "Expected success: $output" }
            if ($output -notmatch [regex]::Escape('"rustc": "rustc 1.92.0 (fixture)"')) { throw "Missing verified version summary: $output" }
        } elseif ($process.ExitCode -eq 0 -or $output -notmatch $case.error) {
            throw "Expected failure matching '$($case.error)', exit=$($process.ExitCode): $output"
        }
        $installs = @($calls | Where-Object { $_.arguments[0] -eq 'install' })
        if ($case.noInstall -and $installs.Count) { throw 'Unexpected install on ready or rejected tools' }
        if ($case.installRust -and !($installs | Where-Object { 'Rustlang.Rustup' -in $_.arguments })) {
            throw 'Did not install rustup'
        }
        Write-Output "PASS $($case.name)"
    } catch {
        $failed++
        Write-Output "FAIL $($case.name): $_"
    } finally {
        # Only this test's exact, verified temporary directory is removed.
        $resolved = [IO.Path]::GetFullPath($root)
        $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
        if (!$resolved.StartsWith($temp, [StringComparison]::OrdinalIgnoreCase) -or
            [IO.Path]::GetFileName($resolved) -notmatch '^touchdeck-setup-[0-9a-f-]+$') { throw 'Unsafe fixture cleanup path' }
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
Write-Output "$($cases.Count - $failed)/$($cases.Count) setup cases passed"
if ($failed) { exit 1 }
