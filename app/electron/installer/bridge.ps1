<#
  Moves Open TouchDeck 1.x to 2.0 (docs/packaging.md, "The bridge to 2.0").

  The 1.x updater runs this from its staging folder after it verified the 2.0 MSI with the updater key, stopped the
  Companion through its shutdown route and quit. It installs the MSI and removes 1.x, or puts 1.x back as it was.

  1. Refuse, changing nothing, unless this is Windows 11, 1.x is installed in its one folder, the MSI still has the
     SHA-256 the updater's signature check read, and 1.x has exited.
  2. Move 1.x's Start menu shortcut into the bridge folder and rename the install folder to "Open TouchDeck.1x-backup".
     The MSI installs to the same folder and refuses to run beside 1.x, so 1.x has to step aside first.
  3. Run the MSI with a progress bar the person can cancel.
  4. If the MSI fails or is cancelled, rename the folder back and restore the shortcut. 1.x starts again as it was.
     Otherwise remove what 1.x left: its uninstall entries, desktop shortcut, Run value, Electron profile and folder.
     User data in the app data folders and Windows Credential Manager is never touched.

  It never runs 1.x's uninstaller: that one always removes "%LOCALAPPDATA%\Programs\Open TouchDeck" and the Start menu
  shortcut "Open TouchDeck", which are the 2.0 MSI's folder and shortcut too.

  The log and result.json go to "bridge" in the app data folder. 1.x reads the result at its next start.
  Exit codes: 0 moved to 2.0; 1 refused, nothing changed; 2 the MSI failed or was cancelled and 1.x is back;
  3 1.x could not be put back (the log says where it is).
#>
param(
  [Parameter(Mandatory = $true)] [string] $Msi,
  [Parameter(Mandatory = $true)] [string] $Sha256,
  [Parameter(Mandatory = $true)] [string] $InstallDir,
  [Parameter(Mandatory = $true)] [string] $DataDir,
  [int] $WaitPid = 0
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3

$OneXFolder = Join-Path $env:LOCALAPPDATA 'Programs\Open TouchDeck'
$Backup = "$OneXFolder.1x-backup"
$BridgeFolder = Join-Path $DataDir 'bridge'
$LogPath = Join-Path $BridgeFolder 'bridge.log'
$ResultPath = Join-Path $BridgeFolder 'result.json'
$MsiLog = Join-Path $BridgeFolder 'msi.log'
$ShortcutBackup = Join-Path $BridgeFolder 'start-menu.lnk'
$StartMenuShortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Open TouchDeck.lnk'
$DesktopShortcut = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Open TouchDeck.lnk'
$RunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$RunValue = 'Open TouchDeck'
$UninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
# Windows Installer's success codes: done, done but a restart finishes it.
$MsiSucceeded = @(0, 1641, 3010)
$MsiCancelled = 1602

function Write-Log([string] $Text) {
  $line = "$([DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ')) $Text"
  [IO.File]::AppendAllText($LogPath, "$line`r`n", [Text.UTF8Encoding]::new($false))
}

function Write-Result([string] $Status, [string] $Detail, $MsiExitCode = $null) {
  $result = [ordered]@{ status = $Status; detail = $Detail; msi_exit_code = $MsiExitCode; msi = (Split-Path -Leaf $Msi)
    finished_at = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ') }
  [IO.File]::WriteAllText($ResultPath, ($result | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
  Write-Log "Result: $Status. $Detail"
}

function Get-FullPath([string] $Path) {
  [IO.Path]::GetFullPath($Path).TrimEnd('\')
}

function Test-SamePath([string] $A, [string] $B) {
  [string]::Equals((Get-FullPath $A), (Get-FullPath $B), [StringComparison]::OrdinalIgnoreCase)
}

function Test-Under([string] $Path, [string] $Folder) {
  $Path -and (Get-FullPath $Path).StartsWith("$(Get-FullPath $Folder)\", [StringComparison]::OrdinalIgnoreCase)
}

function Get-ShortcutTarget([string] $Path) {
  try { (New-Object -ComObject WScript.Shell).CreateShortcut($Path).TargetPath } catch { $null }
}

# Processes started from a folder. Get-Process can't read some system processes' paths; those are never ours.
function Get-ProcessesIn([string] $Folder) {
  @(Get-Process -ErrorAction SilentlyContinue | Where-Object {
    $path = $null
    try { $path = $_.Path } catch { }
    Test-Under $path $Folder
  })
}

function Start-OneX {
  $exe = Join-Path $OneXFolder 'Open TouchDeck.exe'
  if (Test-Path -LiteralPath $exe -PathType Leaf) {
    try { Start-Process -FilePath $exe -WorkingDirectory $OneXFolder; Write-Log 'Started 1.x again.' }
    catch { Write-Log "Could not start 1.x again: $($_.Exception.Message)" }
  }
}

# Renames a folder, retrying while an indexer or antivirus scanner briefly holds a file in it.
function Move-Folder([string] $From, [string] $To) {
  for ($try = 1; ; $try++) {
    try { [IO.Directory]::Move($From, $To); return $true }
    catch {
      if ($try -ge 30) { Write-Log "Could not rename $From to ${To}: $($_.Exception.Message)"; return $false }
      Start-Sleep -Seconds 1
    }
  }
}

function Remove-Folder([string] $Path) {
  for ($try = 1; ; $try++) {
    if (-not (Test-Path -LiteralPath $Path)) { return $true }
    try { Remove-Item -LiteralPath $Path -Recurse -Force; return $true }
    catch {
      if ($try -ge 10) { Write-Log "Could not remove ${Path}: $($_.Exception.Message)"; return $false }
      Start-Sleep -Seconds 1
    }
  }
}

# Refuses with nothing changed. 1.x has already quit, so it starts again.
function Stop-Refused([string] $Detail) {
  Write-Result 'refused' $Detail
  Start-OneX
  exit 1
}

function Test-MsiHash {
  (Get-FileHash -LiteralPath $Msi -Algorithm SHA256).Hash -eq $Sha256.ToUpperInvariant()
}

# Puts 1.x back after the MSI failed or was cancelled: its folder and its Start menu shortcut. Windows Installer has
# already rolled back what the MSI did, but it can leave the folders it created.
function Restore-OneX {
  if (Test-Path -LiteralPath $OneXFolder) {
    if (@(Get-ChildItem -LiteralPath $OneXFolder -Recurse -Force -File).Count -eq 0) {
      if (-not (Remove-Folder $OneXFolder)) { return $false }
    } else {
      $aside = "$OneXFolder.2x-failed-$([DateTime]::UtcNow.ToString('yyyyMMddHHmmss'))"
      Write-Log "The failed MSI left files; moving them to $aside."
      if (-not (Move-Folder $OneXFolder $aside)) { return $false }
    }
  }
  if (-not (Move-Folder $Backup $OneXFolder)) { return $false }
  if ((Test-Path -LiteralPath $ShortcutBackup) -and -not (Test-Path -LiteralPath $StartMenuShortcut)) {
    Move-Item -LiteralPath $ShortcutBackup -Destination $StartMenuShortcut
  }
  return $true
}

# Removes what 1.x registered outside its folder. Each item is removed only when it points at 1.x.
function Remove-OneXLeftovers {
  $uninstaller = "`"$OneXFolder\Uninstall Open TouchDeck.exe`""
  foreach ($key in @(Get-ChildItem -LiteralPath $UninstallKey -ErrorAction SilentlyContinue)) {
    $command = $key.GetValue('UninstallString')
    if ($command -is [string] -and $command.StartsWith($uninstaller, [StringComparison]::OrdinalIgnoreCase)) {
      Remove-Item -LiteralPath $key.PSPath -Recurse -Force
      Write-Log "Removed the 1.x uninstall entry $($key.PSChildName)."
    }
  }
  # electron-builder keeps the install location under HKCU\Software\<app GUID>.
  foreach ($key in @(Get-ChildItem -LiteralPath 'HKCU:\Software' -ErrorAction SilentlyContinue)) {
    if ($key.PSChildName -notmatch '^\{?[0-9A-Fa-f]{8}-([0-9A-Fa-f]{4}-){3}[0-9A-Fa-f]{12}\}?$') { continue }
    $location = $key.GetValue('InstallLocation')
    if ($location -is [string] -and (Test-SamePath $location $OneXFolder)) {
      Remove-Item -LiteralPath $key.PSPath -Recurse -Force
      Write-Log "Removed the 1.x install record $($key.PSChildName)."
    }
  }
  if (Test-Path -LiteralPath $ShortcutBackup) { Remove-Item -LiteralPath $ShortcutBackup -Force }
  if ((Test-Path -LiteralPath $DesktopShortcut) -and
      (Test-SamePath (Get-ShortcutTarget $DesktopShortcut) (Join-Path $OneXFolder 'Open TouchDeck.exe'))) {
    Remove-Item -LiteralPath $DesktopShortcut -Force
    Write-Log 'Removed the 1.x desktop shortcut.'
  }
  # 1.x and 2.0 share the Run value's name and the startup-initialized marker. When 1.x started at sign-in, remove its
  # value and the marker, so 2.0 turns start at sign-in on at its first start, as it does on a new PC. When 1.x had it
  # off, the marker stays and 2.0 leaves it off. Explorer's StartupApproved record belongs to the person and stays.
  $run = Get-Item -LiteralPath $RunKey -ErrorAction SilentlyContinue
  $command = if ($run) { $run.GetValue($RunValue) } else { $null }
  if ($command -is [string] -and $command.StartsWith("`"$OneXFolder\td-companion.exe`"", [StringComparison]::OrdinalIgnoreCase)) {
    Remove-ItemProperty -LiteralPath $RunKey -Name $RunValue
    Remove-Item -LiteralPath (Join-Path $DataDir 'startup-initialized') -Force -ErrorAction SilentlyContinue
    Write-Log 'Start at sign-in was on in 1.x; 2.0 turns it on at its first start.'
  }
  # The Electron window's own profile. The window keeps no user data in it: its session lives in memory.
  if (Remove-Folder (Join-Path $env:APPDATA 'Open TouchDeck')) { Write-Log 'Removed the 1.x window profile.' }
  if (Remove-Folder $Backup) { Write-Log 'Removed the 1.x program files.' } else { return $false }
  return $true
}

New-Item -ItemType Directory -Force -Path $BridgeFolder | Out-Null
Remove-Item -LiteralPath $ResultPath -Force -ErrorAction SilentlyContinue
Write-Log "Bridge to $(Split-Path -Leaf $Msi) started."

# 1. Refuse, changing nothing, unless every precondition holds.
$build = [int](Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion').CurrentBuildNumber
if ($build -lt 22000) { Stop-Refused "Open TouchDeck 2.0 needs Windows 11, and this PC runs build $build." }
if (-not (Test-SamePath $InstallDir $OneXFolder)) { Stop-Refused "Open TouchDeck 1.x is not in its install folder." }
if (-not (Test-Path -LiteralPath (Join-Path $OneXFolder 'Uninstall Open TouchDeck.exe') -PathType Leaf)) {
  Stop-Refused 'Open TouchDeck 1.x is not installed.'
}
if (Test-Path -LiteralPath $Backup) { Stop-Refused "An earlier update left $Backup. Move it away, then retry." }
if (-not (Test-MsiHash)) { Stop-Refused 'The MSI changed after the updater checked its signature.' }

if ($WaitPid -gt 0) { Wait-Process -Id $WaitPid -Timeout 30 -ErrorAction SilentlyContinue }
# The updater already stopped the Companion through its shutdown route, so ending a 1.x process that is still
# running loses nothing: it is the window or a Companion that did not finish exiting.
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while ((Get-ProcessesIn $OneXFolder).Count -gt 0 -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 500 }
$running = Get-ProcessesIn $OneXFolder
if ($running.Count -gt 0) {
  Write-Log "Ending 1.x processes that are still running: $(($running | ForEach-Object { $_.ProcessName }) -join ', ')."
  $running | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 2
  if ((Get-ProcessesIn $OneXFolder).Count -gt 0) { Stop-Refused 'Open TouchDeck 1.x is still running.' }
}

# 2. Move 1.x aside.
if ((Test-Path -LiteralPath $StartMenuShortcut) -and
    (Test-Under (Get-ShortcutTarget $StartMenuShortcut) $OneXFolder)) {
  Move-Item -LiteralPath $StartMenuShortcut -Destination $ShortcutBackup -Force
}
if (-not (Move-Folder $OneXFolder $Backup)) {
  if (Test-Path -LiteralPath $ShortcutBackup) { Move-Item -LiteralPath $ShortcutBackup -Destination $StartMenuShortcut -Force }
  Stop-Refused 'A file in the Open TouchDeck 1.x folder is in use.'
}
Write-Log "Moved 1.x to $Backup."

# 3. Install 2.0. The hash is checked again right before Windows Installer opens the file.
$code = -1
try {
  if (-not (Test-MsiHash)) { throw 'The MSI changed after the updater checked its signature.' }
  $msiexec = Join-Path $env:SystemRoot 'System32\msiexec.exe'
  $process = Start-Process -FilePath $msiexec -Wait -PassThru `
    -ArgumentList @('/i', "`"$Msi`"", '/passive', '/norestart', '/l*v', "`"$MsiLog`"")
  $code = $process.ExitCode
  Write-Log "Windows Installer exited with $code."
} catch {
  Write-Log "Could not run the MSI: $($_.Exception.Message)"
}

# 4. Put 1.x back, or finish the move.
if ($MsiSucceeded -notcontains $code -or -not (Test-Path -LiteralPath (Join-Path $OneXFolder 'td-app.exe') -PathType Leaf)) {
  if ($MsiSucceeded -contains $code) {
    # Windows Installer says 2.0 is installed, but its program is missing. Keep 1.x where it is for support.
    Write-Result 'broken' "The MSI reported success without td-app.exe. Open TouchDeck 1.x is kept in $Backup." $code
    exit 3
  }
  if (-not (Restore-OneX)) {
    Write-Result 'broken' "The MSI did not install, and Open TouchDeck 1.x could not be put back. It is in $Backup." $code
    exit 3
  }
  if ($code -eq $MsiCancelled) {
    Write-Result 'cancelled' 'The update to 2.0 was cancelled. Open TouchDeck 1.x is back.' $code
  } else {
    Write-Result 'failed' "Windows Installer failed with $code. Open TouchDeck 1.x is back; msi.log says why." $code
  }
  Start-OneX
  exit 2
}

$clean = Remove-OneXLeftovers
Remove-Item -LiteralPath $MsiLog -Force -ErrorAction SilentlyContinue
try {
  Start-Process -FilePath (Join-Path $OneXFolder 'td-app.exe') -ArgumentList '--packaged' -WorkingDirectory $OneXFolder
  Write-Log 'Started 2.0.'
} catch { Write-Log "Could not start 2.0: $($_.Exception.Message)" }
if ($clean) {
  Write-Result 'moved' 'Open TouchDeck 2.0 is installed and 1.x is removed.' $code
} else {
  Write-Result 'moved' "Open TouchDeck 2.0 is installed. Some 1.x files stayed in $Backup; delete that folder." $code
}
# The staging folder holds this script and the MSI. Windows lets a running script's folder go.
Remove-Folder (Split-Path -Parent $Msi) | Out-Null
exit 0
