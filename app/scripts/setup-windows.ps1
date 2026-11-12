# Windows source-build prerequisites. Run from any directory with PowerShell 7:
#   pwsh -NoProfile -File app\scripts\setup-windows.ps1
# Dot-sourcing only defines functions (also used by the isolated setup tests).

function Get-InstalledPaths {
    [Environment]::GetEnvironmentVariable('Path', 'Machine')
    [Environment]::GetEnvironmentVariable('Path', 'User')
}

function Update-ProcessPath {
    # Preserve caller choices, then discover installer additions in this process.
    # Never persist PATH or discard process-only entries (for example a version manager).
    $paths = @($env:PATH) + @(Get-InstalledPaths) + @(
        "$env:USERPROFILE\.cargo\bin"
    )
    $env:PATH = (($paths -join ';') -split ';' | Where-Object { $_ } | Select-Object -Unique) -join ';'
}

function Find-Tool($name) {
    # Prefer native/cmd launchers over PowerShell shims requiring a policy change.
    Get-Command $name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
}

function Invoke-Checked($name, [string[]]$arguments, $helpText) {
    $command = Find-Tool $name
    if (-not $command) { throw "$name was not found on PATH. $helpText" }
    $output = @(& $command.Source @arguments)
    $code = $LASTEXITCODE
    if ($code -ne 0) {
        $output | Out-Host
        throw "$name $($arguments -join ' ') failed with exit code $code. $helpText"
    }
    $output
}

function Install-WingetPackage($id, [string[]]$extra = @()) {
    Invoke-Checked 'winget' (@('install', '--id', $id, '--exact', '--source', 'winget', '--silent',
        '--accept-package-agreements', '--accept-source-agreements') + $extra) `
        "Check the installer output for $id; resolve any installation/restart requirement, then rerun setup. Install App Installer from Microsoft if winget is missing." | Out-Host
    Update-ProcessPath
}

function Get-MsvcInstallation {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path -LiteralPath $vswhere) {
        Invoke-Checked $vswhere @('-latest', '-products', '*', '-requires',
            'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-property', 'installationPath') `
            'Repair Visual Studio Installer and rerun setup.'
    }
}

function Invoke-TouchDeckSetup {
    $ErrorActionPreference = 'Stop'
    # Handle native exit codes ourselves, including when the caller enables PS7's preference.
    $PSNativeCommandUseErrorActionPreference = $false
    if (-not $IsWindows) { throw 'This setup script requires Windows and PowerShell 7. See docs/setup.md for Linux.' }
    Update-ProcessPath

    if (-not (Find-Tool 'rustup')) { Install-WingetPackage 'Rustlang.Rustup' }
    $rustHelp = 'Repair rustup (https://rustup.rs), check network access and the installer output, then rerun setup in a new PowerShell.'
    Invoke-Checked 'rustup' @('default', 'stable-msvc') $rustHelp | Out-Host
    Invoke-Checked 'rustup' @('component', 'add', 'clippy', 'rustfmt') $rustHelp | Out-Host
    $rustc = (Invoke-Checked 'rustc' @('--version') $rustHelp) -join "`n"
    if ($rustc -notmatch '^rustc \d+\.\d+\.\d+') { throw "rustc returned no valid version. $rustHelp" }

    $msvc = Get-MsvcInstallation
    if (-not $msvc) {
        Install-WingetPackage 'Microsoft.VisualStudio.2022.BuildTools' @('--override', '--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended')
        $msvc = Get-MsvcInstallation
        if (-not $msvc) { throw 'MSVC tools still not detected. Complete/restart the Visual Studio Installer with Desktop development with C++ and a Windows SDK, then rerun setup.' }
    }
    [pscustomobject]@{ rustc = $rustc.Trim(); msvc = $msvc } | ConvertTo-Json
}

if ($MyInvocation.InvocationName -ne '.') {
    try { Invoke-TouchDeckSetup } catch { Write-Error $_ -ErrorAction Continue; exit 1 }
}
