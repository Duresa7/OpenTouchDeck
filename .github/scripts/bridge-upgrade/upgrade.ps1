<#
.SYNOPSIS
Proves the move from Open TouchDeck 1.x to 2.0 through the bridge on a GitHub-hosted Windows runner (issue #357).

.DESCRIPTION
1. Installs the published 1.2.0, seeds user data in 1.x's formats, starts it and checks it reads the data. Devices,
   the Layout and the Pairing are files plus a Credential Manager key; the Integration is set through 1.x's own API,
   so 1.x writes its file and its secret itself. Then the 1.x updater updates it to the bridge release, a test-signed
   1.3.0 build, the way every 1.x update runs: the NSIS installer with /S --updated --force-run.
2. Tampered MSI: the 1.x updater refuses it and nothing changes.
3. Failing MSI: the bridge runs it, Windows Installer fails and rolls back, and 1.x is back exactly as it was.
4. Good MSI: the bridge installs 2.0 and removes 1.x. 2.0 starts, its Home lists the seeded Device, start at sign-in
   carries over, nothing of 1.x is left, and 2.0's own code reads every piece of seeded data.

The 1.x updater runs from a 1.x checkout (app/scripts/bridge_update.mjs there) against the installed app, with a
loopback stand-in for GitHub. Both MSIs are signed with a throwaway updater key made here and deleted at once.

It writes summary.json and sanitized logs to -ArtifactDir. It refuses to run outside GitHub Actions: it installs
and removes software and writes the real per-user registry and Credential Manager.
#>
[CmdletBinding()]
param(
    # The published 1.2.0 installer, checked against its release's SHA256SUMS by the workflow.
    [Parameter(Mandatory)][string]$OldInstaller,
    [Parameter(Mandatory)][string]$OldVersion,
    # release-assets of the test-signed bridge build: its installer, .sig and release-manifest.json.
    [Parameter(Mandatory)][string]$BridgeAssets,
    [Parameter(Mandatory)][string]$OneXCheckout,
    [Parameter(Mandatory)][string]$Msi,
    [Parameter(Mandatory)][string]$FailingMsi,
    [Parameter(Mandatory)][string]$BridgeData,
    [Parameter(Mandatory)][string]$ArtifactDir,
    # The version the stand-in release offers and the signatures bind.
    [string]$NextVersion = '2.0.0'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($env:GITHUB_ACTIONS -ne 'true') { throw 'upgrade.ps1 only runs on a GitHub Actions runner.' }

$here = $PSScriptRoot
$repo = (Resolve-Path (Join-Path $here '..\..\..')).Path
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\Open TouchDeck'
$backupDir = "$installDir.1x-backup"
$dataDir = Join-Path $env:LOCALAPPDATA 'io.github.duresa7.touchdeck'
$configDir = Join-Path $env:APPDATA 'io.github.duresa7.touchdeck'
$windowProfile = Join-Path $env:APPDATA 'Open TouchDeck'
$bridgeDir = Join-Path $dataDir 'bridge'
$runtimeDir = Join-Path $dataDir 'runtime'
$startMenuShortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Open TouchDeck.lnk'
$desktopShortcut = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Open TouchDeck.lnk'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$uninstallRoot = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
$upgradeCode = '{FA97B489-E1B1-43CA-97AA-6D5FEE4A9836}'
$device = 'a0b1c2d3e4f5'
$deviceName = 'Bridge Desk'
$layoutName = 'Bridge test layout'
$integration = @{ enabled = $false; url = 'http://127.0.0.1:8318'; interval_s = 300 }

$null = New-Item -ItemType Directory -Force -Path $ArtifactDir
$ArtifactDir = (Resolve-Path -LiteralPath $ArtifactDir).Path
$work = New-Item -ItemType Directory -Force -Path (Join-Path $env:RUNNER_TEMP 'bridge-upgrade-work')
$checks = [Collections.Generic.List[string]]::new()
$notes = [ordered]@{}

function Assert-That([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw "FAILED: $Message" }
    $checks.Add($Message)
    Write-Host "ok: $Message"
}

# Replaces the runner's profile folder, user name and PC name, so no artifact names an account or a machine.
function Protect-Text([string]$Text) {
    foreach ($pair in @(@($env:USERPROFILE, '%USERPROFILE%'), @($env:USERNAME, '%USERNAME%'), @($env:COMPUTERNAME, '%COMPUTERNAME%'))) {
        if ($pair[0]) { $Text = $Text -ireplace [regex]::Escape($pair[0]), $pair[1] }
    }
    $Text
}

function Save-Artifact([string]$Name, [string]$Text) {
    [IO.File]::WriteAllText((Join-Path $ArtifactDir $Name), (Protect-Text $Text), [Text.UTF8Encoding]::new($false))
}

function Wait-Until([scriptblock]$Condition, [int]$Seconds, [string]$What) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        try { if (& $Condition) { return } } catch { }
        Start-Sleep -Milliseconds 500
    }
    throw "FAILED: timed out after $Seconds s waiting for $What"
}

function Invoke-Node([string[]]$Arguments) {
    & node @Arguments
    if ($LASTEXITCODE) { throw "node $($Arguments[0]) exited with $LASTEXITCODE" }
}

function Read-Json([string]$Path) { Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json }

function Get-ShortcutInfo([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    $link = (New-Object -ComObject WScript.Shell).CreateShortcut($Path)
    [pscustomobject]@{ Target = $link.TargetPath; Arguments = $link.Arguments }
}

function Get-RunValue {
    (Get-Item -LiteralPath $runKey).GetValue('Open TouchDeck')
}

# 1.x's uninstall entries, found the way the bridge finds them: by the uninstaller they run.
function Get-OneXUninstallKeys {
    @(Get-ChildItem -LiteralPath $uninstallRoot -ErrorAction SilentlyContinue | Where-Object {
        $command = $_.GetValue('UninstallString')
        $command -is [string] -and $command.StartsWith("`"$installDir\Uninstall Open TouchDeck.exe`"", [StringComparison]::OrdinalIgnoreCase)
    } | ForEach-Object { $_.PSChildName })
}

# electron-builder's install record: HKCU\Software\<GUID> with InstallLocation.
function Get-OneXInstallRecords {
    @(Get-ChildItem -LiteralPath 'HKCU:\Software' -ErrorAction SilentlyContinue | Where-Object {
        $_.PSChildName -match '^\{?[0-9A-Fa-f]{8}-([0-9A-Fa-f]{4}-){3}[0-9A-Fa-f]{12}\}?$' -and
            ([string]$_.GetValue('InstallLocation')).TrimEnd('\') -ieq $installDir
    } | ForEach-Object { $_.PSChildName })
}

function Get-ProcessesIn([string]$Folder) {
    @(Get-Process -ErrorAction SilentlyContinue | Where-Object {
        $path = $null
        try { $path = $_.Path } catch { }
        $path -and $path.StartsWith("$Folder\", [StringComparison]::OrdinalIgnoreCase)
    })
}

# The products installed with 2.0's UpgradeCode, and whether each is installed for the current user only.
function Get-TwoXProducts {
    $installer = New-Object -ComObject WindowsInstaller.Installer
    $related = $installer.GetType().InvokeMember('RelatedProducts', 'GetProperty', $null, $installer, @($upgradeCode))
    $count = [int]$related.GetType().InvokeMember('Count', 'GetProperty', $null, $related, $null)
    $products = @(for ($i = 0; $i -lt $count; $i++) {
        $code = [string]$related.GetType().InvokeMember('Item', 'GetProperty', $null, $related, @($i))
        [pscustomobject]@{
            Code       = $code
            State      = [int]$installer.GetType().InvokeMember('ProductState', 'GetProperty', $null, $installer, @($code))
            Assignment = [string]$installer.GetType().InvokeMember('ProductInfo', 'GetProperty', $null, $installer, @($code, 'AssignmentType'))
        }
    })
    $null = [Runtime.InteropServices.Marshal]::ReleaseComObject($installer)
    $products
}

# A hash of every file under the user data folders, except the runtime, bridge and cache folders the apps manage.
function Get-DataFiles {
    $files = [ordered]@{}
    foreach ($root in @($dataDir, $configDir)) {
        if (-not (Test-Path -LiteralPath $root)) { continue }
        foreach ($file in Get-ChildItem -LiteralPath $root -Recurse -File -Force) {
            $relative = $file.FullName.Substring($root.Length).TrimStart('\')
            if ($relative -match '^(runtime|bridge|cache)\\' -or $relative -like '*.tmp') { continue }
            $files["$(Split-Path -Leaf $root)\$relative"] = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
        }
    }
    $files
}

# The Credential Manager targets 1.x and 2.0 keep their secrets under.
function Get-Credentials {
    @(cmdkey /list | ForEach-Object { if ($_ -match 'Target:\s*(?:LegacyGeneric:target=)?(.*touchdeck.*)$') { $Matches[1].Trim() } } | Sort-Object -Unique)
}

function Read-OneX([string]$Name) {
    $out = Join-Path $work "$Name.json"
    Invoke-Node @((Join-Path $here 'onex_api.mjs'), $OneXCheckout, 'read', $out)
    Copy-Item -LiteralPath $out -Destination (Join-Path $ArtifactDir "$Name.json")
    Read-Json $out
}

# Closes the 1.x window the way a person does. The Companion runs on. The real updater quits the window itself.
function Close-OneXWindow {
    foreach ($window in @(Get-Process -Name 'Open TouchDeck' -ErrorAction SilentlyContinue | Where-Object MainWindowHandle -ne 0)) {
        $null = $window.CloseMainWindow()
    }
    try { Wait-Until { @(Get-Process -Name 'Open TouchDeck' -ErrorAction SilentlyContinue).Count -eq 0 } 20 'the 1.x window to close' }
    catch { Get-Process -Name 'Open TouchDeck' -ErrorAction SilentlyContinue | Stop-Process -Force }
}

function Wait-OneXRunning {
    if (@(Get-Process -Name 'Open TouchDeck' -ErrorAction SilentlyContinue).Count -eq 0) {
        Start-Process -FilePath (Join-Path $installDir 'Open TouchDeck.exe') -WorkingDirectory $installDir
    }
    Wait-Until { (Test-Path -LiteralPath (Join-Path $runtimeDir 'companion.json')) } 60 'the 1.x Companion runtime file'
    Wait-Until { Invoke-Node @((Join-Path $here 'onex_api.mjs'), $OneXCheckout, 'read', (Join-Path $work 'probe.json')); $true } 60 'the 1.x Companion API'
}

function Assert-OneXIntact([string]$When) {
    Assert-That (Test-Path -LiteralPath (Join-Path $installDir 'Open TouchDeck.exe')) "$When`: 1.x is in its install folder"
    Assert-That (Test-Path -LiteralPath (Join-Path $installDir 'Uninstall Open TouchDeck.exe')) "$When`: the 1.x uninstaller is in place"
    Assert-That (-not (Test-Path -LiteralPath (Join-Path $installDir 'td-app.exe'))) "$When`: no 2.0 program is in the install folder"
    Assert-That (-not (Test-Path -LiteralPath $backupDir)) "$When`: no backup folder is left"
    Assert-That (@(Get-ChildItem -LiteralPath (Split-Path $installDir) | Where-Object Name -like 'Open TouchDeck.2x-failed-*').Count -eq 0) "$When`: no failed-install folder is left"
    Assert-That (@(Get-OneXUninstallKeys).Count -eq 1) "$When`: the 1.x uninstall entry is in place"
    $link = Get-ShortcutInfo $startMenuShortcut
    Assert-That ($null -ne $link -and $link.Target -ieq (Join-Path $installDir 'Open TouchDeck.exe')) "$When`: the Start menu shortcut opens 1.x"
    Assert-That ([string](Get-RunValue) -like "`"$installDir\td-companion.exe`"*") "$When`: start at sign-in still runs the 1.x Companion"
    Assert-That (@(Get-TwoXProducts | Where-Object State -eq 5).Count -eq 0) "$When`: Windows Installer lists no 2.0 product"
}

function Assert-DataKept([string]$When, $Before) {
    $after = Get-DataFiles
    $missing = @($Before.Keys | Where-Object { -not $after.Contains($_) })
    Assert-That ($missing.Count -eq 0) "$When`: every user data file is still there ($($Before.Count) files)"
    $changed = @($Before.Keys | Where-Object { $after.Contains($_) -and $after[$_] -ne $Before[$_] })
    $notes["$When changed files"] = $changed
    foreach ($seeded in @("io.github.duresa7.touchdeck\layouts\$device\landscape.json")) {
        Assert-That ($after[$seeded] -eq $Before[$seeded]) "$When`: the seeded Layout file is unchanged"
    }
    $credentials = @(Get-Credentials)
    foreach ($target in $script:credentialsBefore) {
        Assert-That ($credentials -contains $target) "$When`: Credential Manager still holds $target"
    }
}

# Runs the 1.x updater through the bridge. Returns the updater's result and, when it started bridge.ps1, a handle on
# it taken while it still waits for 1.x to exit, so its exit code can be read.
function Invoke-Bridge([string]$Scenario, [string]$Dir, [string]$Name) {
    $out = Join-Path $work "update-$Name.json"
    Invoke-Node @((Join-Path $OneXCheckout 'app\scripts\bridge_update.mjs'), $Scenario, $OneXVersion, $Dir, $out)
    $result = Read-Json $out
    Copy-Item -LiteralPath $out -Destination (Join-Path $ArtifactDir "update-$Name.json")
    $helper = $null
    if ($result.PSObject.Properties.Name -contains 'helper_pid') {
        $helper = [Diagnostics.Process]::GetProcessById([int]$result.helper_pid)
        $null = $helper.Handle
    }
    [pscustomobject]@{ Result = $result; Helper = $helper }
}

function Wait-Helper($Run, [string]$Name) {
    Assert-That ($null -ne $Run.Helper) "$Name`: the updater started bridge.ps1"
    Assert-That ($Run.Helper.WaitForExit(600000)) "$Name`: bridge.ps1 finished within 10 minutes"
    $code = $Run.Helper.ExitCode
    $notes["$Name bridge exit code"] = $code
    foreach ($log in 'bridge.log', 'msi.log') {
        $path = Join-Path $bridgeDir $log
        if (Test-Path -LiteralPath $path) { Save-Artifact "$Name-$log" (Get-Content -LiteralPath $path -Raw) }
    }
    $code
}

# A signed copy of an MSI under the release name, with the next.json bridge_update.mjs reads.
function New-Release([string]$Source, [string]$Name, [string]$Key) {
    $dir = New-Item -ItemType Directory -Force -Path (Join-Path $work $Name)
    $msiName = "OpenTouchDeck-$NextVersion-windows-x64.msi"
    Copy-Item -LiteralPath $Source -Destination (Join-Path $dir $msiName)
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ''
    Invoke-Node @((Join-Path $repo '.github\release-signer\cli.mjs'), 'sign', '--app-version', $NextVersion, '--private-key-path', $Key, (Join-Path $dir $msiName))
    $next = [ordered]@{
        version            = $NextVersion
        msi                = $msiName
        signature          = (Get-Content -LiteralPath (Join-Path $dir "$msiName.sig") -Raw).Trim()
        updater_public_key = (Get-Content -LiteralPath "$Key.pub" -Raw).Trim()
    }
    [IO.File]::WriteAllText((Join-Path $dir 'next.json'), ($next | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    $dir.FullName
}

$OneXVersion = ''
$windows = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
$summary = [ordered]@{
    windows      = "$($windows.ProductName), build $($windows.CurrentBuildNumber)"
    runnerImage  = "$env:ImageOS $env:ImageVersion"
    oneXVersion  = $OldVersion
    nextVersion  = $NextVersion
    checks       = $checks
    notes        = $notes
    result       = 'failed'
}
try {
    Assert-That ([int]$windows.CurrentBuildNumber -ge 22000) "the runner is Windows 11 or later (build $($windows.CurrentBuildNumber))"
    foreach ($path in @($installDir, $dataDir, $configDir, $windowProfile)) {
        Assert-That (-not (Test-Path -LiteralPath $path)) "the runner starts with no $(Split-Path -Leaf $path) folder"
    }

    # --- 1. Install 1.x and seed user data --------------------------------------------------------------------
    $manifest = Read-Json (Join-Path $BridgeAssets 'release-manifest.json')
    $OneXVersion = [string]$manifest.version
    $summary.oneXVersion = "$OldVersion, then $OneXVersion"
    $process = Start-Process -FilePath $OldInstaller -ArgumentList '/S' -Wait -PassThru
    Assert-That ($process.ExitCode -eq 0) "$OldVersion installs silently"
    Assert-That (@(Get-OneXUninstallKeys).Count -eq 1) "$OldVersion has one uninstall entry"
    $notes['1.x uninstall entries'] = @(Get-OneXUninstallKeys)
    $notes['1.x install records'] = @(Get-OneXInstallRecords)
    $notes['1.x desktop shortcut'] = [bool](Get-ShortcutInfo $desktopShortcut)

    # Devices, the Layout and the Pairing go in before 1.x first starts, as files in 1.x's formats and the Pairing key
    # in Credential Manager. 1.x reading them back below proves they are 1.x data.
    Copy-Item -Path (Join-Path $here 'seed\*') -Destination (New-Item -ItemType Directory -Force -Path $dataDir) -Recurse -Force
    & $BridgeData seed-pairing-key $device
    if ($LASTEXITCODE) { throw "bridge_data seed-pairing-key exited with $LASTEXITCODE" }

    # Started as the Start menu shortcut starts it. The window starts the Companion, which turns on start at sign-in.
    Start-Process -FilePath (Join-Path $installDir 'Open TouchDeck.exe') -WorkingDirectory $installDir
    Wait-OneXRunning
    Wait-Until { [string](Get-RunValue) -like "`"$installDir\td-companion.exe`"*" } 60 '1.x to turn on start at sign-in'
    Assert-That (Test-Path -LiteralPath (Join-Path $dataDir 'startup-initialized')) '1.x marked start at sign-in as set up'
    Invoke-Node @((Join-Path $here 'onex_api.mjs'), $OneXCheckout, 'seed', (Join-Path $work 'seed.json'))

    $before = Read-OneX 'onex-before'
    $seen = @($before.devices | Where-Object id -eq $device)
    Assert-That ($seen.Count -eq 1 -and $seen[0].name -eq $deviceName) "1.x lists the seeded Device $deviceName"
    Assert-That ($seen[0].layout_name -eq $layoutName -and $seen[0].layout_pages -eq 4) '1.x reads the seeded Layout'
    Assert-That (@($before.pairings | Where-Object device -eq $device).Count -eq 1) '1.x lists the seeded Pairing'
    Assert-That ($before.integration.config.url -eq $integration.url -and $before.integration.config.interval_s -eq $integration.interval_s) '1.x stored the Integration settings'
    Assert-That ($before.integration.secrets.management_key.set -eq $true) '1.x stored the Integration secret'
    Assert-That ($before.startup.at_sign_in -eq $true) '1.x starts at sign-in'

    # 1.2.0 updates to the bridge release through the real 1.x updater against a loopback latest.json.
    $update = New-Item -ItemType Directory -Force -Path (Join-Path $work 'to-bridge')
    Copy-Item -LiteralPath (Join-Path $BridgeAssets $manifest.installer) -Destination $update
    $next = [ordered]@{
        version = $OneXVersion; installer = [string]$manifest.installer; updater_public_key = [string]$manifest.updater_public_key
        signature = (Get-Content -LiteralPath (Join-Path $BridgeAssets "$($manifest.installer).sig") -Raw).Trim()
    }
    [IO.File]::WriteAllText((Join-Path $update 'next.json'), ($next | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    $toBridge = Join-Path $work 'update-to-bridge.json'
    Invoke-Node @((Join-Path $OneXCheckout 'app\scripts\lifecycle_update.mjs'), 'good', $OldVersion, $update.FullName, $toBridge)
    Copy-Item -LiteralPath $toBridge -Destination $ArtifactDir
    # The installer closes the old window itself, as it does when the window quits slowly.
    Assert-That ((Read-Json $toBridge).quit -eq $true) "the $OldVersion updater verified the bridge release and started its installer"
    Wait-Until { (Get-Item -LiteralPath (Join-Path $uninstallRoot @(Get-OneXUninstallKeys)[0])).GetValue('DisplayVersion') -eq $OneXVersion } 300 "the $OneXVersion installer"
    Wait-Until { Test-Path -LiteralPath (Join-Path $installDir 'resources\bridge.ps1') } 60 'bridge.ps1 in the installed resources'
    $checks.Add("$OldVersion updated to $OneXVersion, which ships bridge.ps1")
    Wait-OneXRunning
    $updated = Read-OneX 'onex-after-update'
    Assert-That ((@($updated.devices | Where-Object id -eq $device)[0]).layout_name -eq $layoutName -and $updated.integration.secrets.management_key.set -eq $true) "$OneXVersion reads the data $OldVersion kept"

    $dataBefore = Get-DataFiles
    $script:credentialsBefore = @(Get-Credentials)
    $notes['user data files'] = @($dataBefore.Keys)
    $notes['Credential Manager targets'] = $script:credentialsBefore
    Assert-That ($script:credentialsBefore.Count -ge 2) 'Credential Manager holds the Pairing key and the Integration secret'

    # The throwaway updater key signs both MSIs, then goes.
    $key = Join-Path $work 'throwaway-updater.key'
    Invoke-Node @((Join-Path $repo '.github\release-signer\cli.mjs'), 'generate', '--write-keys', $key)
    try {
        $goodRelease = New-Release $Msi 'good' $key
        $failingRelease = New-Release $FailingMsi 'failing' $key
    } finally { Remove-Item -LiteralPath $key -Force -ErrorAction SilentlyContinue }

    # --- 2. A tampered MSI is refused -------------------------------------------------------------------------
    $run = Invoke-Bridge 'tampered' $goodRelease 'tampered'
    Assert-That ($run.Result.check.state -eq 'available' -and $run.Result.check.bridge -eq $true -and $run.Result.check.version -eq $NextVersion) "tampered: Windows 11 is offered $NextVersion"
    Assert-That ($run.Result.download.state -eq 'failed' -and $run.Result.download.reason -eq 'signature') 'tampered: the signature check refuses the MSI'
    Assert-That ($null -eq $run.Helper) 'tampered: bridge.ps1 never starts'
    Assert-That (@(Get-ChildItem -LiteralPath $runtimeDir -Filter 'update-*' -Directory).Count -eq 0) 'tampered: the download is deleted'
    Assert-OneXIntact 'tampered'
    Wait-OneXRunning

    # --- 3. An MSI that fails puts 1.x back -------------------------------------------------------------------
    # The 1.x window stays open here, so bridge.ps1 has to end it: as when a window hangs instead of quitting.
    $run = Invoke-Bridge 'good' $failingRelease 'failing'
    Assert-That ($run.Result.download.state -eq 'ready') 'failing: the signed MSI downloads and verifies'
    Assert-That ($run.Result.quit -eq $true) 'failing: the updater stops the Companion and quits'
    $code = Wait-Helper $run 'failing'
    Assert-That ($code -eq 2) "failing: bridge.ps1 reports the MSI failed and 1.x is back (exit $code)"
    Assert-That ((Get-Content -LiteralPath (Join-Path $bridgeDir 'bridge.log') -Raw) -match 'Windows Installer exited with 1603') 'failing: Windows Installer failed with 1603'
    Assert-OneXIntact 'failing'
    Assert-That (-not (Test-Path -LiteralPath 'HKCU:\Software\Open TouchDeck bridge check')) 'failing: Windows Installer rolled the failed MSI back'
    Wait-OneXRunning
    # 1.x shows why at its start, then deletes the result.
    Wait-Until { -not (Test-Path -LiteralPath (Join-Path $bridgeDir 'result.json')) } 60 '1.x to read the bridge result'
    $checks.Add('failing: 1.x restarted and read the bridge result')
    $after = Read-OneX 'onex-after-failure'
    Assert-That ((@($after.devices | Where-Object id -eq $device)[0]).layout_name -eq $layoutName) 'failing: 1.x still reads the seeded Device and Layout'
    Assert-That ($after.integration.secrets.management_key.set -eq $true) 'failing: 1.x still has the Integration secret'
    Assert-DataKept 'failing' $dataBefore

    # --- 4. The real MSI moves the user to 2.0 ----------------------------------------------------------------
    # This time the window is closed first, as the real updater quits it before bridge.ps1 starts.
    Close-OneXWindow
    $run = Invoke-Bridge 'good' $goodRelease 'good'
    Assert-That ($run.Result.download.state -eq 'ready') 'good: the signed MSI downloads and verifies'
    $code = Wait-Helper $run 'good'
    Assert-That ($code -eq 0) "good: bridge.ps1 reports 2.0 installed (exit $code)"
    $result = Read-Json (Join-Path $bridgeDir 'result.json')
    Assert-That ($result.status -eq 'moved') "good: the bridge result says moved: $($result.detail)"

    $products = @(Get-TwoXProducts | Where-Object State -eq 5)
    Assert-That ($products.Count -eq 1) 'good: Windows Installer lists 2.0 as installed'
    Assert-That ($products[0].Assignment -eq '0') 'good: 2.0 is installed for the current user, not the whole PC'
    $left = @(Get-ChildItem -LiteralPath $installDir -Recurse -Force | ForEach-Object { $_.FullName.Substring($installDir.Length + 1) })
    $notes['install folder after the move'] = $left
    Assert-That (($left -join ',') -eq 'td-app.exe') 'good: the install folder holds td-app.exe and nothing of 1.x'
    Assert-That (-not (Test-Path -LiteralPath $backupDir)) 'good: the 1.x program files are gone'
    Assert-That (@(Get-ChildItem -LiteralPath (Split-Path $installDir) | Where-Object Name -like 'Open TouchDeck.*').Count -eq 0) 'good: no other Open TouchDeck folder is left in Programs'
    Assert-That (@(Get-OneXUninstallKeys).Count -eq 0) 'good: the 1.x uninstall entry is gone'
    Assert-That (@(Get-OneXInstallRecords).Count -eq 0) "good: the 1.x install record under HKCU\Software is gone"
    $uninstallNames = @(Get-ChildItem -LiteralPath $uninstallRoot | ForEach-Object { [string]$_.GetValue('DisplayName') } | Where-Object { $_ -like 'Open TouchDeck*' })
    Assert-That ($uninstallNames.Count -eq 1) "good: Installed apps lists one Open TouchDeck ($($uninstallNames -join ', '))"
    $link = Get-ShortcutInfo $startMenuShortcut
    Assert-That ($null -ne $link -and $link.Target -ieq (Join-Path $installDir 'td-app.exe') -and $link.Arguments -eq '--packaged') 'good: the Start menu shortcut opens 2.0'
    Assert-That (@(Get-ChildItem -LiteralPath ([Environment]::GetFolderPath('Programs')) -Filter 'Open TouchDeck*').Count -eq 1) 'good: the Start menu has one Open TouchDeck entry'
    Assert-That (-not (Test-Path -LiteralPath $desktopShortcut)) 'good: no 1.x desktop shortcut is left'
    Assert-That (-not (Test-Path -LiteralPath $windowProfile)) 'good: the 1.x window profile is gone'
    Assert-That (@(Get-ChildItem -LiteralPath $runtimeDir -Filter 'update-*' -Directory -ErrorAction SilentlyContinue).Count -eq 0) 'good: the staged MSI and bridge.ps1 are gone'
    Assert-That (@(Get-ProcessesIn $backupDir).Count -eq 0 -and @(Get-Process -Name 'Open TouchDeck', 'td-companion' -ErrorAction SilentlyContinue).Count -eq 0) 'good: no 1.x process runs'

    # The bridge started 2.0 with --packaged, which turns start at sign-in on again for the new program.
    Wait-Until { @(Get-ProcessesIn $installDir | Where-Object ProcessName -eq 'td-app').Count -eq 1 } 60 '2.0 to start'
    $checks.Add('good: 2.0 is running')
    Wait-Until { [string](Get-RunValue) -eq "`"$installDir\td-app.exe`" --packaged" } 60 '2.0 to turn on start at sign-in'
    $checks.Add('good: start at sign-in carried over and now starts 2.0')
    Assert-That (Test-Path -LiteralPath (Join-Path $dataDir 'startup-initialized')) 'good: 2.0 marked start at sign-in as set up'

    # 2.0's Home lists the seeded Device, read from the data 1.x left.
    $app = @(Get-ProcessesIn $installDir | Where-Object ProcessName -eq 'td-app')[0]
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
    $root = [Windows.Automation.AutomationElement]::RootElement
    $byPid = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty, $app.Id)
    $names = @()
    Wait-Until {
        $window = $root.FindFirst([Windows.Automation.TreeScope]::Children, $byPid)
        if ($null -eq $window) { return $false }
        $script:names = @($window.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition) |
            ForEach-Object { $_.Current.Name } | Where-Object { $_ })
        @($script:names | Where-Object { $_ -like "*$deviceName*" }).Count -gt 0
    } 60 "2.0's window to list $deviceName"
    $notes['2.0 window names'] = $script:names
    $checks.Add("good: 2.0's Home lists $deviceName")

    # 2.0's own code reads every piece of seeded data. Stop the app first, so only one Companion holds the folder.
    $app | Stop-Process -Force
    Wait-Until { @(Get-ProcessesIn $installDir).Count -eq 0 } 30 '2.0 to stop'
    $reportText = & $BridgeData report
    if ($LASTEXITCODE) { throw "bridge_data report exited with $LASTEXITCODE" }
    Save-Artifact 'twox-report.json' ($reportText -join "`n")
    $report = ($reportText -join "`n") | ConvertFrom-Json
    $seen = @($report.devices | Where-Object id -eq $device)
    Assert-That ($seen.Count -eq 1 -and $seen[0].name -eq $deviceName) "2.0 reads the Device $deviceName"
    Assert-That ($seen[0].layouts.landscape.name -eq $layoutName -and @($seen[0].layouts.landscape.pages).Count -eq 4) '2.0 reads its Layout'
    $pairing = @($report.pairings | Where-Object device -eq $device)
    Assert-That ($pairing.Count -eq 1) '2.0 reads the Pairing'
    Assert-That ($pairing[0].key -eq 'loads') "2.0 loads the Pairing key from Credential Manager ($($pairing[0].key))"
    $config = $report.integrations.cliproxyapi.config
    Assert-That ($config.url -eq $integration.url -and $config.interval_s -eq $integration.interval_s -and $config.enabled -eq $false) '2.0 reads the Integration settings'
    Assert-That ($report.integrations.cliproxyapi.secrets.management_key.set -eq $true) '2.0 reads the Integration secret from Credential Manager'
    Assert-That ($report.app_settings.automatic_update_checks -eq $true) "2.0 reads its own app settings"
    Assert-DataKept 'good' $dataBefore

    $summary.result = 'passed'
} catch {
    $summary.error = Protect-Text $_.Exception.Message
    foreach ($log in 'bridge.log', 'msi.log', 'result.json') {
        $path = Join-Path $bridgeDir $log
        if (Test-Path -LiteralPath $path) { Save-Artifact "last-$log" (Get-Content -LiteralPath $path -Raw) }
    }
    throw
} finally {
    Save-Artifact 'summary.json' ($summary | ConvertTo-Json -Depth 6)
    if ($env:GITHUB_STEP_SUMMARY) {
        $lines = @("## Bridge upgrade: $($summary.result)", '', "$($summary.windows); runner image $($summary.runnerImage)", '', "1.x $OneXVersion to $NextVersion", '')
        $lines += $checks | ForEach-Object { "- $_" }
        if ($summary.Contains('error')) { $lines += @('', "**$($summary.error)**") }
        Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Value (Protect-Text ($lines -join "`n")) -Encoding utf8
    }
}
