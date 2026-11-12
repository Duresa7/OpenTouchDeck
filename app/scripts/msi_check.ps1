<#
.SYNOPSIS
Proves the Open TouchDeck MSI on a GitHub-hosted Windows runner: either that it installs for the current user and
uninstalls cleanly, or that an older Windows refuses it.

.DESCRIPTION
-Expect install            Checks the package's properties, that it refuses to install beside Open TouchDeck 1.x,
                           installs it silently, checks the installed program and the per-user registration, then
                           uninstalls it and checks nothing is left.
-Expect refuse-old-windows Runs on a Windows older than Windows 11 (build 22000) and checks the install is refused
                           with the message that names Windows 11, and that nothing was installed.

It writes summary.json and the msiexec logs to -ArtifactDir, with the runner's user name, PC name and profile
folder replaced. It refuses to run outside GitHub Actions: it writes the real per-user registry.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Msi,
    [Parameter(Mandatory)][ValidateSet('install', 'refuse-old-windows')][string]$Expect,
    [Parameter(Mandatory)][string]$ArtifactDir,
    # The version of td-companion, such as 2.0.0. Needed for -Expect install.
    [string]$Version,
    # SHA-256 of the td-companion.exe the package was built from. Needed for -Expect install.
    [string]$ExeSha256
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($env:GITHUB_ACTIONS -ne 'true') { throw 'msi_check.ps1 only runs on a GitHub Actions runner.' }

$productName = 'Open TouchDeck'
$manufacturer = 'Duresa Kadi'
$upgradeCode = '{FA97B489-E1B1-43CA-97AA-6D5FEE4A9836}'
$windows11Build = 22000
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\Open TouchDeck'
$installedExe = Join-Path $installDir 'td-companion.exe'
$appKey = 'HKCU:\Software\Open TouchDeck'

$msiPath = (Resolve-Path -LiteralPath $Msi).Path
$null = New-Item -ItemType Directory -Force -Path $ArtifactDir
$ArtifactDir = (Resolve-Path -LiteralPath $ArtifactDir).Path
$checks = [Collections.Generic.List[string]]::new()

function Assert-That([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw "FAILED: $Message" }
    $checks.Add($Message)
    Write-Host "ok: $Message"
}

# Replaces the runner's profile folder, user name and PC name, so no log names an account or a machine.
function Protect-Text([string]$Text) {
    foreach ($pair in @(@($env:USERPROFILE, '%USERPROFILE%'), @($env:USERNAME, '%USERNAME%'), @($env:COMPUTERNAME, '%COMPUTERNAME%'))) {
        if ($pair[0]) { $Text = $Text -ireplace [regex]::Escape($pair[0]), $pair[1] }
    }
    $Text
}

# Runs msiexec silently with a log. Returns its exit code and the log text; keeps a cleaned copy of the log.
function Invoke-Msiexec([string]$Name, [string[]]$Arguments) {
    $log = Join-Path $env:RUNNER_TEMP "msiexec-$Name.log"
    $process = Start-Process -FilePath msiexec.exe -ArgumentList ($Arguments + @('/qn', '/norestart', '/l*', "`"$log`"")) -Wait -PassThru
    $text = if (Test-Path -LiteralPath $log) { Get-Content -LiteralPath $log -Raw } else { '' }
    Set-Content -LiteralPath (Join-Path $ArtifactDir "$Name.log") -Value (Protect-Text $text) -Encoding utf8
    Write-Host "msiexec $Name exited with $($process.ExitCode)"
    [pscustomobject]@{ ExitCode = $process.ExitCode; Log = $text }
}

function Get-ComProperty([object]$Object, [string]$Name, [object[]]$Arguments) {
    $Object.GetType().InvokeMember($Name, 'GetProperty', $null, $Object, $Arguments)
}
function Invoke-ComMethod([object]$Object, [string]$Name, [object[]]$Arguments) {
    $Object.GetType().InvokeMember($Name, 'InvokeMethod', $null, $Object, $Arguments)
}

# The package's Property table and the "no elevation needed" bit of its summary, read through Windows Installer.
function Read-Package([string]$Path) {
    $installer = New-Object -ComObject WindowsInstaller.Installer
    $database = Invoke-ComMethod $installer 'OpenDatabase' @($Path, 0)
    $view = Invoke-ComMethod $database 'OpenView' @('SELECT `Property`, `Value` FROM `Property`')
    $null = Invoke-ComMethod $view 'Execute' $null
    $properties = @{}
    while ($record = Invoke-ComMethod $view 'Fetch' $null) {
        $properties[(Get-ComProperty $record 'StringData' @(1))] = Get-ComProperty $record 'StringData' @(2)
    }
    $summary = Get-ComProperty $database 'SummaryInformation' @(0)
    # Summary property 15 is the word count: bit 8 marks a package that never asks for elevation.
    $wordCount = [int](Get-ComProperty $summary 'Property' @(15))
    # Let go of the file before msiexec opens it.
    foreach ($object in @($summary, $view, $database, $installer)) { $null = [Runtime.InteropServices.Marshal]::ReleaseComObject($object) }
    [pscustomobject]@{ Properties = $properties; NoElevation = [bool]($wordCount -band 8) }
}

# What Windows Installer says about a product for the current user, and where its uninstall entry is.
# State 5 is installed and -1 is unknown. AssignmentType 0 is "for the current user" and 1 is "for the whole PC".
function Get-Registration([string]$ProductCode) {
    $installer = New-Object -ComObject WindowsInstaller.Installer
    $state = [int](Get-ComProperty $installer 'ProductState' @($ProductCode))
    $info = @{}
    if ($state -eq 5) {
        foreach ($name in 'AssignmentType', 'InstalledProductName', 'VersionString', 'Publisher') {
            $info[$name] = [string](Get-ComProperty $installer 'ProductInfo' @($ProductCode, $name))
        }
    }
    $null = [Runtime.InteropServices.Marshal]::ReleaseComObject($installer)
    [pscustomobject]@{
        State      = $state
        Info       = $info
        UserKey    = Test-Path -LiteralPath "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$ProductCode"
        MachineKey = [bool](@('HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall') |
            Where-Object { Test-Path -LiteralPath "$_\$ProductCode" })
    }
}

$windows = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
$build = [int]$windows.CurrentBuildNumber
$summary = [ordered]@{
    expect       = $Expect
    msi          = Split-Path -Leaf $msiPath
    msiSha256    = (Get-FileHash -LiteralPath $msiPath -Algorithm SHA256).Hash.ToLowerInvariant()
    windows      = $windows.ProductName
    windowsBuild = $build
    runnerImage  = "$env:ImageOS $env:ImageVersion"
}
Write-Host "Windows: $($windows.ProductName), build $build, runner image $($summary.runnerImage)"

$package = Read-Package $msiPath
$productCode = $package.Properties['ProductCode']
$summary.productCode = $productCode
$summary.productName = $package.Properties['ProductName']
$summary.productVersion = $package.Properties['ProductVersion']

if ($Expect -eq 'refuse-old-windows') {
    Assert-That ($build -lt $windows11Build) "This runner is older than Windows 11: build $build is below $windows11Build."
    $result = Invoke-Msiexec 'install-refused' @('/i', "`"$msiPath`"")
    Assert-That ($result.ExitCode -eq 1603) "msiexec refuses the install with exit code 1603 (got $($result.ExitCode))."
    Assert-That ($result.Log -match 'Open TouchDeck needs Windows 11') 'The log gives the reason: Open TouchDeck needs Windows 11.'
    Assert-That (-not (Test-Path -LiteralPath $installDir)) 'No install folder was created.'
    $registration = Get-Registration $productCode
    Assert-That ($registration.State -eq -1 -and -not $registration.UserKey -and -not $registration.MachineKey) 'Windows Installer registered nothing.'
    $summary.refusedExitCode = $result.ExitCode
    $summary.refusedMessage = ([regex]::Match($result.Log, 'Open TouchDeck needs Windows 11[^\r\n]*')).Value
}
else {
    if (-not $Version -or $ExeSha256 -notmatch '^[a-f0-9]{64}$') { throw '-Expect install needs -Version and -ExeSha256.' }
    Assert-That ($build -ge $windows11Build) "This runner counts as Windows 11: build $build is at least $windows11Build."

    # The package itself.
    $numbers = ($Version -split '[-+]')[0]
    Assert-That ($package.Properties['ProductName'] -ceq $productName) "ProductName is '$productName'."
    Assert-That ($package.Properties['Manufacturer'] -ceq $manufacturer) "Manufacturer is '$manufacturer'."
    Assert-That ($package.Properties['ProductVersion'] -ceq $numbers) "ProductVersion is $numbers, the program's version."
    Assert-That ($package.Properties['UpgradeCode'] -ieq $upgradeCode) 'UpgradeCode is the committed one.'
    Assert-That (-not $package.Properties.ContainsKey('ALLUSERS')) 'The package sets no ALLUSERS: it installs for the current user.'
    Assert-That $package.NoElevation 'The package is marked as never needing elevation.'
    Assert-That (-not (Test-Path -LiteralPath $installDir)) 'The install folder does not exist before the install.'

    # Beside Open TouchDeck 1.x: refuse, and leave its folder alone.
    $null = New-Item -ItemType Directory -Force -Path $installDir
    $marker = Join-Path $installDir 'Uninstall Open TouchDeck.exe'
    Set-Content -LiteralPath $marker -Value 'stand-in for the Open TouchDeck 1.x uninstaller'
    $result = Invoke-Msiexec 'install-beside-1x' @('/i', "`"$msiPath`"")
    Assert-That ($result.ExitCode -eq 1603) "With Open TouchDeck 1.x in the folder, msiexec refuses with exit code 1603 (got $($result.ExitCode))."
    Assert-That ($result.Log -match 'Open TouchDeck 1\.x is installed on this PC') 'The log gives the reason: Open TouchDeck 1.x is installed.'
    Assert-That ((Test-Path -LiteralPath $marker) -and -not (Test-Path -LiteralPath $installedExe)) 'The 1.x folder is untouched.'
    Remove-Item -LiteralPath $installDir -Recurse -Force

    # Install.
    $result = Invoke-Msiexec 'install' @('/i', "`"$msiPath`"")
    Assert-That ($result.ExitCode -eq 0) "msiexec installs with exit code 0 (got $($result.ExitCode))."
    Assert-That (Test-Path -LiteralPath $installedExe) 'td-companion.exe is in %LOCALAPPDATA%\Programs\Open TouchDeck.'
    $installedHash = (Get-FileHash -LiteralPath $installedExe -Algorithm SHA256).Hash.ToLowerInvariant()
    Assert-That ($installedHash -eq $ExeSha256) 'The installed td-companion.exe is byte for byte the built one.'
    & (Join-Path $PSScriptRoot 'check_exe.ps1') -Path $installedExe -Subsystem gui -Version $Version | Out-Host
    Assert-That $true "The installed program carries ProductName '$productName', version $Version and the GUI subsystem."
    $registration = Get-Registration $productCode
    Assert-That ($registration.State -eq 5) 'Windows Installer lists the product as installed.'
    Assert-That ($registration.Info['AssignmentType'] -eq '0') 'It is installed for the current user, not for the whole PC (AssignmentType 0).'
    Assert-That ($registration.Info['InstalledProductName'] -ceq $productName -and $registration.Info['VersionString'] -ceq $numbers) "It is listed as '$productName' $numbers."
    Assert-That ($registration.Info['Publisher'] -ceq $manufacturer) "Its publisher is '$manufacturer'."
    Assert-That (-not $registration.MachineKey) 'It has no uninstall entry under HKEY_LOCAL_MACHINE.'
    Assert-That ((Get-ItemProperty -LiteralPath $appKey).InstallFolder.TrimEnd('\') -ieq $installDir) 'The component registered its install folder under HKCU.'
    $summary.installExitCode = $result.ExitCode
    $summary.installedExeSha256 = $installedHash
    $summary.assignmentType = $registration.Info['AssignmentType']
    $summary.uninstallEntryUnderHkcu = $registration.UserKey

    # Uninstall.
    $result = Invoke-Msiexec 'uninstall' @('/x', $productCode)
    Assert-That ($result.ExitCode -eq 0) "msiexec uninstalls with exit code 0 (got $($result.ExitCode))."
    Assert-That (-not (Test-Path -LiteralPath $installDir)) 'The install folder is gone.'
    $registration = Get-Registration $productCode
    Assert-That ($registration.State -eq -1 -and -not $registration.UserKey -and -not $registration.MachineKey) 'Windows Installer no longer lists the product.'
    Assert-That (-not (Test-Path -LiteralPath $appKey)) 'The component removed its registry key.'
    $summary.uninstallExitCode = $result.ExitCode
}

$summary.checks = $checks
$json = $summary | ConvertTo-Json -Depth 4
Set-Content -LiteralPath (Join-Path $ArtifactDir 'summary.json') -Value $json -Encoding utf8
Write-Host $json
if ($env:GITHUB_STEP_SUMMARY) {
    $lines = @("### MSI check: $Expect", '', "$($windows.ProductName), build $build, runner image $($summary.runnerImage)", '') + ($checks | ForEach-Object { "- $_" })
    $lines -join "`n" | Out-File -FilePath $env:GITHUB_STEP_SUMMARY -Append -Encoding utf8
}
