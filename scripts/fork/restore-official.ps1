<#
.SYNOPSIS
  Puts the official Hangar.AI executables back, undoing install.ps1.

.DESCRIPTION
  Copies backups\official\hangar-ai.exe and hangar-webview.exe over the installed ones. The
  official build then updates itself to the latest release as usual. Your data is left as it
  is, since the official app reads the same folder; pass -DataBackup to also put back a data
  folder that install.ps1 saved (the current one is backed up first).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\fork\restore-official.ps1
  powershell -ExecutionPolicy Bypass -File scripts\fork\restore-official.ps1 -DataBackup backups\20261117-230000
#>
param(
  [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Hangar.AI"),
  [string]$DataDir = (Join-Path $env:APPDATA "com.simnjs.iabench"),
  [string]$DataBackup = ""
)
$ErrorActionPreference = "Stop"

$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$official = Join-Path $repo "backups\official"
$exes = "hangar-ai.exe", "hangar-webview.exe"

$running = @(tasklist /fi "imagename eq hangar-ai.exe" /nh /fo csv | Select-String "hangar-ai.exe")
if ($running.Count -gt 0) { throw "Hangar is running. Close it first, then run this again." }
foreach ($exe in $exes) {
  if (-not (Test-Path (Join-Path $official $exe))) { throw "$official\$exe is missing: nothing to restore." }
}

foreach ($exe in $exes) { Copy-Item (Join-Path $official $exe) $InstallDir -Force }
Remove-Item (Join-Path $InstallDir ".hangar-fork") -ErrorAction SilentlyContinue
Write-Host "Official Hangar $(Get-Content (Join-Path $official 'VERSION')) is back in $InstallDir"

if ($DataBackup) {
  $source = Join-Path (Resolve-Path $DataBackup).Path "data"
  if (-not (Test-Path (Join-Path $source "state.json"))) { throw "$source holds no state.json." }
  $keep = Join-Path $repo ("backups\" + (Get-Date -Format "yyyyMMdd-HHmmss") + "-before-restore\data")
  robocopy $DataDir $keep /E /R:1 /W:1 /NFL /NDL /NJH /NJS /NP | Out-Null
  if ($LASTEXITCODE -ge 8) { throw "Backing up the current data failed (robocopy exit $LASTEXITCODE)." }
  robocopy $source $DataDir /MIR /R:1 /W:1 /NFL /NDL /NJH /NJS /NP | Out-Null
  if ($LASTEXITCODE -ge 8) { throw "Restoring $source failed (robocopy exit $LASTEXITCODE)." }
  Write-Host "Data restored from $source (the data it replaced is in $keep)"
}
