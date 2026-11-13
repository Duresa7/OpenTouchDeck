<#
.SYNOPSIS
Reads the version resource and the Windows subsystem of a built program (td-app.exe), prints
them as JSON, and fails when they are not what the build must carry. The resource must name the file it is in.

.DESCRIPTION
build.rs writes the version resource and main.rs picks the subsystem: GUI for a release build, console for a
debug build. The release workflow runs this on a GitHub-hosted Windows runner against the program it just built.

.EXAMPLE
pwsh app/scripts/check_exe.ps1 -Path app/target/release/td-app.exe -Subsystem gui
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Path,
    [Parameter(Mandatory)][ValidateSet('gui', 'console')][string]$Subsystem,
    # The Cargo version of the workspace crates. Read from td-companion's Cargo.toml when omitted.
    [string]$Version
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$file = (Resolve-Path -LiteralPath $Path).Path
if (-not $Version) {
    $manifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot '../crates/td-companion/Cargo.toml') -Raw
    if ($manifest -notmatch '(?m)^\[package\]\r?\n(?:(?!\[)[^\n]*\n)*?version\s*=\s*"([^"]+)"') {
        throw 'td-companion/Cargo.toml has no package version.'
    }
    $Version = $Matches[1]
}

# The subsystem is a 16-bit field 68 bytes into the PE optional header: 2 is GUI, 3 is console.
$bytes = [IO.File]::ReadAllBytes($file)
$pe = [BitConverter]::ToInt32($bytes, 0x3c)
if ([Text.Encoding]::ASCII.GetString($bytes, $pe, 4) -ne "PE`0`0") { throw "$file is not a Windows program." }
$subsystemValue = [BitConverter]::ToUInt16($bytes, $pe + 24 + 68)
$subsystemName = switch ($subsystemValue) { 2 { 'gui' } 3 { 'console' } default { "unknown ($subsystemValue)" } }

$info = [Diagnostics.FileVersionInfo]::GetVersionInfo($file)
$facts = [ordered]@{
    file                 = Split-Path -Leaf $file
    subsystem            = $subsystemName
    ProductName          = $info.ProductName
    FileDescription      = $info.FileDescription
    CompanyName          = $info.CompanyName
    OriginalFilename     = $info.OriginalFilename
    InternalName         = $info.InternalName
    LegalCopyright       = $info.LegalCopyright
    ProductVersion       = $info.ProductVersion
    FileVersion          = $info.FileVersion
    ProductVersionNumber = "$($info.ProductMajorPart).$($info.ProductMinorPart).$($info.ProductBuildPart).$($info.ProductPrivatePart)"
    FileVersionNumber    = "$($info.FileMajorPart).$($info.FileMinorPart).$($info.FileBuildPart).$($info.FilePrivatePart)"
}
$json = $facts | ConvertTo-Json
Write-Output $json
if ($env:GITHUB_STEP_SUMMARY) {
    "### $($facts.file)`n`n``````json`n$json`n```````n" | Out-File -FilePath $env:GITHUB_STEP_SUMMARY -Append -Encoding utf8
}

# 2.0.0-rc.1 carries the numbers 2.0.0.0.
$numbers = "$(($Version -split '[-+]')[0]).0"
$expected = [ordered]@{
    subsystem            = $Subsystem
    ProductName          = 'Open TouchDeck'
    FileDescription      = 'Open TouchDeck'
    CompanyName          = 'Duresa Kadi'
    OriginalFilename     = $facts.file
    InternalName         = [IO.Path]::GetFileNameWithoutExtension($facts.file)
    LegalCopyright       = 'Copyright Duresa Kadi'
    ProductVersion       = $Version
    FileVersion          = $Version
    ProductVersionNumber = $numbers
    FileVersionNumber    = $numbers
}
$wrong = @(foreach ($name in $expected.Keys) {
        if ($facts[$name] -cne $expected[$name]) { "$name is '$($facts[$name])', not '$($expected[$name])'" }
    })
if ($wrong.Count) { throw "$($facts.file): $($wrong -join '; ')" }
Write-Output "$($facts.file) carries version $Version and the $Subsystem subsystem."
