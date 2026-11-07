param(
    [string]$AssetsDir,
    [string]$LifecycleDir,
    [string]$ArtifactDir = "$env:RUNNER_TEMP/installer-lifecycle"
)

# Installation changes the real user profile and registry: never run on an operator's PC.
# Keep this guard before path resolution, directory creation, or any other side effect.
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted') {
    Write-Error 'Installer lifecycle requires GITHUB_ACTIONS=true and RUNNER_ENVIRONMENT=github-hosted.' -ErrorAction Continue
    exit 1
}

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Installer lifecycle requires Windows.' }
if (-not $AssetsDir -or -not $LifecycleDir) { throw 'Pass -AssetsDir and -LifecycleDir.' }
$release = Get-Content -LiteralPath (Join-Path $AssetsDir 'release-manifest.json') -Raw | ConvertFrom-Json
$next = Get-Content -LiteralPath (Join-Path $LifecycleDir 'next.json') -Raw | ConvertFrom-Json
if (-not $release.test_signing) { throw 'The lifecycle test only runs test-signed candidates.' }
$installer = (Resolve-Path -LiteralPath (Join-Path $AssetsDir $release.installer)).Path
$nextInstaller = (Resolve-Path -LiteralPath (Join-Path $LifecycleDir $next.installer)).Path
$driver = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'lifecycle_update.mjs')).Path
$null = New-Item -ItemType Directory -Path $ArtifactDir -Force
$logPath = Join-Path $ArtifactDir 'lifecycle.log'
$summaryPath = Join-Path $ArtifactDir 'summary.json'
$scratch = Join-Path $env:RUNNER_TEMP ('touchdeck-lifecycle-' + [guid]::NewGuid().ToString('N'))
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\Open TouchDeck'
$appExe = Join-Path $installDir 'Open TouchDeck.exe'
$companionExe = Join-Path $installDir 'td-companion.exe'
$uninstaller = Join-Path $installDir 'Uninstall Open TouchDeck.exe'
$dataDir = Join-Path $env:LOCALAPPDATA 'io.github.duresa7.touchdeck'
$configDir = Join-Path $env:APPDATA 'io.github.duresa7.touchdeck'
$windowDir = Join-Path $env:APPDATA 'Open TouchDeck'
$runtimeFile = Join-Path $dataDir 'runtime\companion.json'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$children = [Collections.Generic.List[object]]::new()
$checks = [Collections.Generic.List[object]]::new()
$summary = [ordered]@{
    status = 'running'; scenario = 'preconditions'; version = $release.version; next = $next.version
    checks = $checks; remaining = @(); failure = $null
}

function Protect-Log([string]$Text) {
    foreach ($name in @('LOCALAPPDATA', 'APPDATA', 'RUNNER_TEMP', 'GITHUB_WORKSPACE', 'USERPROFILE', 'HOME', 'COMPUTERNAME')) {
        $value = [Environment]::GetEnvironmentVariable($name)
        if ($value) {
            $Text = $Text.Replace($value, "%$name%").Replace($value.Replace('\', '/'), "%$name%")
        }
    }
    $Text = [regex]::Replace($Text, 'td[ps]_[A-Za-z0-9_-]{43}', '<token>')
    return [regex]::Replace($Text, '(?i)[a-z]:[\\/]Users[\\/][^\\/\s"'']+', '%USERPROFILE%')
}

function Write-Log([string]$Text) {
    $line = Protect-Log $Text
    Add-Content -LiteralPath $logPath -Value $line -Encoding utf8
    Write-Host $line
}

function Assert-That([bool]$Condition, [string]$Message) {
    $checks.Add([ordered]@{ scenario = $summary.scenario; assertion = $Message; passed = $Condition })
    if (-not $Condition) { throw $Message }
    Write-Log "PASS: $Message"
}

function Wait-Until([scriptblock]$Condition, [int]$Seconds) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (& $Condition) { return $true }
        Start-Sleep -Milliseconds 250
    }
    return [bool](& $Condition)
}

function Same-Bytes([string]$Path, [byte[]]$Expected) {
    return (Test-Path -LiteralPath $Path -PathType Leaf) -and
        [Convert]::ToBase64String([IO.File]::ReadAllBytes($Path)) -ceq [Convert]::ToBase64String($Expected)
}

function Start-Child([string]$Exe, [string[]]$Arguments = @()) {
    $start = [Diagnostics.ProcessStartInfo]::new($Exe)
    $start.UseShellExecute = $false
    $start.WorkingDirectory = $scratch
    $start.RedirectStandardInput = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($arg in $Arguments) { $start.ArgumentList.Add($arg) }
    $process = [Diagnostics.Process]::Start($start)
    $child = [pscustomobject]@{
        Process = $process; Out = $process.StandardOutput.ReadToEndAsync(); Err = $process.StandardError.ReadToEndAsync()
        Logged = $false
    }
    $children.Add($child)
    $process.StandardInput.Close()
    return $child
}

function Finish-Child($Child, [string]$Label, [int]$Expected = 0, [int]$TimeoutMs = 180000) {
    if (-not $Child.Process.WaitForExit($TimeoutMs)) {
        $Child.Process.Kill($true)
        $Child.Process.WaitForExit()
        throw "$Label timed out."
    }
    Write-Log "$Label exit code: $($Child.Process.ExitCode)"
    Write-Output-Of $Child
    Assert-That ($Child.Process.ExitCode -eq $Expected) "$Label exits with $Expected."
}

function Write-Output-Of($Child) {
    # A process the child started can hold its output pipes open; never wait on them for long.
    foreach ($stream in @($Child.Out, $Child.Err)) {
        if ($stream.Wait(10000)) { Write-Log $stream.Result } else { Write-Log '(output still held open by a descendant)' }
    }
    $Child.Logged = $true
}

function Get-Field($Value, [string]$Name) {
    if ($null -ne $Value -and $Value.PSObject.Properties[$Name]) { return $Value.$Name }
    return $null
}

function Get-Running([string]$Path) {
    $name = [IO.Path]::GetFileNameWithoutExtension($Path)
    @(Get-Process -Name $name -ErrorAction SilentlyContinue | Where-Object {
        $_.Path -and [string]::Equals($_.Path, $Path, [StringComparison]::OrdinalIgnoreCase)
    })
}

function Get-UninstallEntries {
    $root = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
    if (Test-Path -LiteralPath $root) {
        Get-ChildItem -LiteralPath $root | ForEach-Object {
            $entry = Get-ItemProperty -LiteralPath $_.PSPath
            if ($entry.PSObject.Properties['DisplayName'] -and $entry.DisplayName -like 'Open TouchDeck*') { $entry }
        }
    }
}

function Get-InstalledVersion {
    $entries = @(Get-UninstallEntries)
    if ($entries.Count -eq 1) { return $entries[0].DisplayVersion }
    return $null
}

function Get-RunValue {
    $values = Get-ItemProperty -LiteralPath $runKey -ErrorAction SilentlyContinue
    if ($values -and $values.PSObject.Properties['Open TouchDeck']) { return $values.'Open TouchDeck' }
    return $null
}

function Get-Field($Object, [string]$Name) {
    # Strict mode refuses a missing property, and a missing field must read as absent.
    if ($null -eq $Object) { return $null }
    $property = $Object.PSObject.Properties[$Name]
    if ($property) { return $property.Value }
    return $null
}

function Assert-Installed([string]$Version) {
    foreach ($relative in @('Open TouchDeck.exe', 'td-companion.exe', 'resources\app.asar', 'resources\firmware\touchdeck.tdfw',
                            'resources\install-manifest.json', 'resources\LICENSE.txt', 'Uninstall Open TouchDeck.exe')) {
        Assert-That (Test-Path -LiteralPath (Join-Path $installDir $relative) -PathType Leaf) "The install contains $relative."
    }
    $manifest = Get-Content -LiteralPath (Join-Path $installDir 'resources\install-manifest.json') -Raw | ConvertFrom-Json
    $firmwareHash = (Get-FileHash -LiteralPath (Join-Path $installDir 'resources\firmware\touchdeck.tdfw') -Algorithm SHA256).Hash
    Assert-That ($manifest.firmware.sha256 -eq $firmwareHash) 'The installed Firmware bundle matches the install manifest.'
    Assert-That ((Get-Field $manifest.firmware.manifest 'radio') -ne $true) 'The installed Firmware bundle is the normal build.'
    # The radio build ships embedded in td-companion.exe, never as a file next to the normal build.
    $radio = Get-Field $manifest 'radio_firmware'
    Assert-That ($null -ne $radio) 'The install manifest records the radio build.'
    Assert-That ((Get-Field (Get-Field $radio 'manifest') 'radio') -eq $true) 'The recorded radio build carries the radio mark.'
    Assert-That ("$(Get-Field $radio 'sha256')" -match '^[0-9a-f]{64}$') 'The recorded radio build has a SHA-256.'
    Assert-That ((Get-Field $radio 'embedded_in') -eq 'td-companion.exe') 'The radio build is recorded as embedded in the Companion.'
    $loose = @(Get-ChildItem -LiteralPath (Join-Path $installDir 'resources\firmware') -File | Where-Object Name -ne 'touchdeck.tdfw')
    Assert-That ($loose.Count -eq 0) 'The install holds no Firmware file besides the normal build.'
    $entries = @(Get-UninstallEntries)
    Assert-That ($entries.Count -eq 1) 'Exactly one per-user uninstall entry exists.'
    Assert-That ($entries[0].DisplayVersion -eq $Version) "The uninstall entry reports version $Version."
    Assert-That ((Get-Item -LiteralPath $appExe).VersionInfo.ProductVersion -like "$Version*") "Open TouchDeck.exe is version $Version."
}

function Write-AppProcesses([string]$Label) {
    $rows = @(Get-CimInstance Win32_Process -Filter "Name = 'Open TouchDeck.exe' OR Name = 'td-companion.exe'" | ForEach-Object {
        $type = if ($_.CommandLine -match '--type=(\S+)') { $Matches[1] } else { 'main' }
        $handle = (Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue)?.MainWindowHandle
        "$($_.Name) pid=$($_.ProcessId) parent=$($_.ParentProcessId) type=$type window=$handle"
    })
    Write-Log "$Label processes: $($rows -join '; ')"
}

function Wait-Started([string]$Label) {
    Assert-That (Wait-Until { @(Get-Running $companionExe).Count -eq 1 -and (Test-Path -LiteralPath $runtimeFile) } 60) "$Label starts the installed Companion."
    Assert-That (Wait-Until { @(Get-Running $appExe).Count -gt 0 } 30) "$Label starts the Open TouchDeck window."
}

function Invoke-Driver([string]$Scenario) {
    $result = Join-Path $ArtifactDir "update-$Scenario.json"
    Finish-Child (Start-Child 'node' @($driver, $Scenario, (Get-InstalledVersion), $LifecycleDir, $result)) "Updater driver ($Scenario)"
    $value = Get-Content -LiteralPath $result -Raw | ConvertFrom-Json
    Write-Log "Updater driver ($Scenario): $(ConvertTo-Json $value -Compress -Depth 5)"
    return $value
}

function Wait-Uninstalled {
    # The NSIS uninstaller copies itself to %TEMP% and lets the original exit; wait for the copy.
    $null = Wait-Until { -not @(Get-Process | Where-Object { $_.Path -like '*\~nsu*.tmp\*' }).Count -and
        -not (Test-Path -LiteralPath $uninstaller) -and -not @(Get-UninstallEntries).Count } 120
}

function Write-Sentinels {
    $written = @{}
    foreach ($dir in @($dataDir, $configDir, $windowDir)) {
        $null = New-Item -ItemType Directory -Path $dir -Force
        $path = Join-Path $dir 'lifecycle-sentinel.bin'
        $bytes = [byte[]]@(0, 1, 13, 10, 127, 128, 255)
        [IO.File]::WriteAllBytes($path, $bytes)
        $written[$path] = $bytes
    }
    return $written
}

try {
    Assert-That (-not (Test-Path -LiteralPath $installDir)) 'The runner starts without an Open TouchDeck install.'
    Assert-That (@(Get-UninstallEntries).Count -eq 0) 'The runner starts without an Open TouchDeck uninstall entry.'
    Assert-That (-not (Test-Path -LiteralPath $dataDir) -and -not (Test-Path -LiteralPath $configDir)) 'The runner starts without Open TouchDeck user data.'
    Assert-That ($null -eq (Get-RunValue)) 'The runner starts without the Run entry.'
    $null = New-Item -ItemType Directory -Path $scratch

    $summary.scenario = 'install'
    Finish-Child (Start-Child $installer @('/S')) 'Silent install'
    Assert-Installed $release.version
    $window = Start-Process -FilePath $appExe -WorkingDirectory $scratch -PassThru
    Wait-Started 'Opening the app'
    Write-AppProcesses "Companion ready (browser pid $($window.Id))"
    Assert-That ((Get-RunValue) -ceq "`"$companionExe`" --packaged") 'The Companion registers the per-user Run entry for the installed Companion.'
    $companion = @(Get-Running $companionExe)[0].Id
    Assert-That (Wait-Until { $window.Refresh(); $window.MainWindowHandle -ne [IntPtr]::Zero } 60) 'The Open TouchDeck window appears.'
    Write-AppProcesses 'Window shown'
    Assert-That ($window.CloseMainWindow()) 'The window accepts a close request.'
    Assert-That (Wait-Until { -not @(Get-Running $appExe).Count } 30) 'Closing the window ends Electron.'
    Assert-That (@(Get-Running $companionExe).Id -contains $companion) 'The Companion keeps running after the window closes.'

    $summary.scenario = 'tampered-update'
    $tampered = Invoke-Driver 'tampered'
    Assert-That ($tampered.check.state -eq 'available' -and $tampered.check.version -eq $next.version) 'The updater offers the newer test-signed build.'
    Assert-That ($tampered.download.state -eq 'failed' -and (Get-Field $tampered.download 'reason') -eq 'signature') 'The updater refuses an installer whose bytes do not match the signature.'
    Assert-That ($null -eq (Get-Field $tampered 'installer_pid')) 'No installer runs after a bad signature.'
    Assert-That (@(Get-Running $companionExe).Id -contains $companion) 'The Companion keeps running after a refused update.'
    Assert-That ((Get-InstalledVersion) -eq $release.version) 'A refused update leaves the installed version.'

    $summary.scenario = 'update'
    $sentinels = Write-Sentinels
    $good = Invoke-Driver 'good'
    Assert-That ($good.download.state -eq 'ready') 'The updater verifies the signed newer installer.'
    $installerPid = Get-Field $good 'installer_pid'
    Assert-That ((Get-Field $good 'quit') -eq $true -and $installerPid -gt 0) 'The updater launches the installer and quits.'
    Assert-That ((@($good.args) -join ' ') -ceq '/S --updated --force-run') 'The updater passes the installer the silent update flags.'
    Assert-That (Wait-Until { -not (Get-Process -Id $installerPid -ErrorAction SilentlyContinue) } 300) 'The update installer finishes.'
    Assert-That (Wait-Until { (Get-InstalledVersion) -eq $next.version } 60) "The update installs $($next.version)."
    Assert-Installed $next.version
    Assert-That (-not (Get-Process -Id $companion -ErrorAction SilentlyContinue)) 'The old Companion stopped for the update.'
    Wait-Started 'The update'
    Assert-That ((Get-RunValue) -ceq "`"$companionExe`" --packaged") 'The update keeps the Run entry.'
    foreach ($path in $sentinels.Keys) {
        Assert-That (Same-Bytes $path $sentinels[$path]) "The update keeps $(Split-Path -Leaf (Split-Path -Parent $path))\lifecycle-sentinel.bin."
    }

    $summary.scenario = 'downgrade'
    $companion = @(Get-Running $companionExe)[0].Id
    Finish-Child (Start-Child $installer @('/S')) 'Silent install of the older build' 5
    Assert-That ((Get-InstalledVersion) -eq $next.version) 'The older installer leaves the newer version installed.'
    Assert-That (@(Get-Running $companionExe).Id -contains $companion) 'A refused downgrade leaves the Companion running.'

    $summary.scenario = 'uninstall'
    Finish-Child (Start-Child $uninstaller @('/S')) 'Silent uninstall'
    Wait-Uninstalled
    Assert-That (-not @(Get-Running $companionExe).Count -and -not @(Get-Running $appExe).Count) 'Uninstall stops the Companion and the window.'
    Assert-That (-not (Test-Path -LiteralPath $runtimeFile)) 'The Companion shut down cleanly: its runtime file is gone.'
    foreach ($relative in @('Open TouchDeck.exe', 'td-companion.exe', 'resources', 'Uninstall Open TouchDeck.exe')) {
        Assert-That (-not (Test-Path -LiteralPath (Join-Path $installDir $relative))) "Uninstall removes $relative."
    }
    Assert-That (@(Get-UninstallEntries).Count -eq 0) 'Uninstall removes the uninstall entry.'
    Assert-That ($null -eq (Get-RunValue)) 'Uninstall removes the Run entry.'
    Assert-That (-not (Test-Path -LiteralPath (Join-Path $dataDir 'startup-initialized'))) 'Uninstall resets the startup marker for a later install.'
    foreach ($path in $sentinels.Keys) {
        Assert-That (Same-Bytes $path $sentinels[$path]) "Uninstall keeps user data by default: $(Split-Path -Leaf (Split-Path -Parent $path))."
    }

    $summary.scenario = 'uninstall-delete-data'
    Finish-Child (Start-Child $nextInstaller @('/S')) 'Silent reinstall'
    Assert-Installed $next.version
    Finish-Child (Start-Child $uninstaller @('/S', '--delete-app-data')) 'Silent uninstall that deletes app data'
    Wait-Uninstalled
    Assert-That (-not (Test-Path -LiteralPath $appExe) -and -not @(Get-UninstallEntries).Count) 'The second uninstall removes the app.'
    foreach ($dir in @($dataDir, $configDir, $windowDir)) {
        Assert-That (-not (Test-Path -LiteralPath $dir)) "--delete-app-data removes $(Split-Path -Leaf $dir)."
    }
    $summary.status = 'passed'
} catch {
    $summary.status = 'failed'
    $summary.failure = Protect-Log $_.Exception.Message
    Write-Log "FAIL: $($summary.failure)"
    try { Write-AppProcesses 'At failure' } catch { Write-Log "Process listing failed: $($_.Exception.Message)" }
} finally {
    foreach ($path in @($installDir, $dataDir, $configDir, $windowDir)) {
        if (Test-Path -LiteralPath $path) {
            $summary.remaining += @(Get-ChildItem -LiteralPath $path -Force -Recurse | ForEach-Object { Protect-Log $_.FullName })
        }
    }
    Write-Log ('Remaining entries: ' + (ConvertTo-Json -InputObject $summary.remaining -Compress))
    foreach ($path in @($appExe, $companionExe)) { Get-Running $path | Stop-Process -Force -ErrorAction SilentlyContinue }
    foreach ($child in $children) {
        if (-not $child.Process.HasExited) { $child.Process.Kill($true); $child.Process.WaitForExit() }
        if (-not $child.Logged) { Write-Output-Of $child }
        $child.Process.Dispose()
    }
    $summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $summaryPath -Encoding utf8
}
if ($summary.status -ne 'passed') { exit 1 }
