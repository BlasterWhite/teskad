<#
.SYNOPSIS
  Puts this fork's build in place of the installed Hangar.AI, keeping every workspace.

.DESCRIPTION
  Hangar keeps its workspaces, board, memory and settings in %APPDATA%\com.simnjs.iabench,
  a folder named after the app identifier, which this fork keeps unchanged. The fork therefore
  reads the very data the official app wrote: nothing is converted or copied over. The script:

    1. refuses while Hangar is running (its exe would be locked and state.json in use);
    2. backs up the data folder to backups\<time>\data, and the official executables, the
       first time, to backups\official (for restore-official.ps1);
    3. copies target\<profile>\hangar-ai.exe and hangar-webview.exe over the installed ones,
       so the shortcuts and every agent's MCP config, which name that path, keep working;
    4. repoints a Start Menu shortcut that names a stale copy of Hangar in another app's
       package cache (an install run from inside a packaged app lands there).

  scripts\fork\restore-official.ps1 undoes it.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\fork\install.ps1
#>
param(
  [string]$BuildProfile = "dist",
  [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Hangar.AI"),
  [string]$DataDir = (Join-Path $env:APPDATA "com.simnjs.iabench"),
  [string]$StartMenuLink = (Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Hangar.AI.lnk"),
  # What the shortcut should open, as Windows will see it (differs from InstallDir only when
  # InstallDir is given as a \\localhost\C$\... path).
  [string]$ShortcutTarget = ""
)
$ErrorActionPreference = "Stop"

$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$build = Join-Path $repo "target\$BuildProfile"
$backups = Join-Path $repo "backups"
$exes = "hangar-ai.exe", "hangar-webview.exe"
if (-not $ShortcutTarget) { $ShortcutTarget = Join-Path $InstallDir "hangar-ai.exe" }

# 1. Hangar must be closed.
$running = @(tasklist /fi "imagename eq hangar-ai.exe" /nh /fo csv | Select-String "hangar-ai.exe")
if ($running.Count -gt 0) {
  throw "Hangar is running ($($running.Count) hangar-ai.exe processes). Close it first, then run this again."
}
foreach ($exe in $exes) {
  if (-not (Test-Path (Join-Path $build $exe))) { throw "$build\$exe is missing: run scripts/fork/build.sh first." }
}
if (-not (Test-Path (Join-Path $InstallDir "hangar-ai.exe"))) { throw "No Hangar install at $InstallDir" }

# 2. Backups.
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$dest = Join-Path $backups $stamp
New-Item -ItemType Directory -Force -Path $dest | Out-Null
if (Test-Path $DataDir) {
  robocopy $DataDir (Join-Path $dest "data") /E /R:1 /W:1 /NFL /NDL /NJH /NJS /NP | Out-Null
  if ($LASTEXITCODE -ge 8) { throw "Backing up $DataDir failed (robocopy exit $LASTEXITCODE)." }
}
$marker = Join-Path $InstallDir ".hangar-fork"
$official = Join-Path $backups "official"
if (-not (Test-Path $marker)) {
  # What is installed now is the official build: keep it for restore-official.ps1.
  New-Item -ItemType Directory -Force -Path $official | Out-Null
  foreach ($exe in $exes) { Copy-Item (Join-Path $InstallDir $exe) $official -Force }
  (Get-Item (Join-Path $official "hangar-ai.exe")).VersionInfo.ProductVersion |
    Set-Content (Join-Path $official "VERSION")
} else {
  # An earlier fork build: kept beside this run's data backup.
  foreach ($exe in $exes) { Copy-Item (Join-Path $InstallDir $exe) $dest -Force }
}

# 3. The fork's executables, at the installed path.
foreach ($exe in $exes) { Copy-Item (Join-Path $build $exe) $InstallDir -Force }
$commit = (git -C $repo rev-parse --short HEAD) 2>$null
"fork build $commit ($BuildProfile), installed $stamp" | Set-Content $marker

# 4. A Start Menu shortcut into a package cache opens a stale copy, not this install.
$fixed = $false
if (Test-Path $StartMenuLink) {
  $shell = New-Object -ComObject WScript.Shell
  $link = $shell.CreateShortcut($StartMenuLink)
  if ($link.TargetPath -like "*\Packages\*\LocalCache\*") {
    Copy-Item $StartMenuLink $dest -Force
    $link.TargetPath = $ShortcutTarget
    $link.WorkingDirectory = Split-Path $ShortcutTarget
    $link.IconLocation = "$ShortcutTarget,0"
    $link.Save()
    $fixed = $true
  }
}

Write-Host "Installed the fork ($commit, $BuildProfile) into $InstallDir"
Write-Host "Data backup:  $dest\data"
if (Test-Path $official) { Write-Host "Official exes: $official (version $(Get-Content (Join-Path $official 'VERSION')))" }
if ($fixed) { Write-Host "Start Menu shortcut now opens $ShortcutTarget (old one saved in $dest)" }
Write-Host "Start Hangar as usual; your workspaces are where they were."
