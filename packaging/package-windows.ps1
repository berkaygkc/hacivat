# Windows: unsigned per-user NSIS installer + minisign .minisig. Run on the Windows host (not yet exercised).
# usage: pwsh packaging/package-windows.ps1 <version>
param([Parameter(Mandatory = $true)][string]$Version)
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")
$Root = (Get-Location).Path
$Key = if ($env:UPDATE_KEY) { $env:UPDATE_KEY } else { Join-Path $HOME ".hacivat-keys/update.key" }
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root "target" }
$Out = Join-Path $TargetDir "packager/$Version"
if (Test-Path $Out) { Remove-Item -Recurse -Force $Out }
New-Item -ItemType Directory -Force $Out | Out-Null
$env:HACIVAT_SPIKE_VERSION = $Version
cargo build --release -p spike-harman
Copy-Item (Join-Path $TargetDir "release/spike-harman.exe") $Out
$Cfg = Join-Path $Root "packaging/.Packager.generated.toml"
(Get-Content packaging/Packager.toml) -replace "@VERSION@", $Version | Set-Content $Cfg
try {
  cargo packager -c $Cfg --formats nsis --out-dir $Out
} finally { Remove-Item -Force $Cfg -ErrorAction SilentlyContinue }
Get-ChildItem "$Out/*-setup.exe" | ForEach-Object {
  minisign -S -s $Key -m $_.FullName -x "$($_.FullName).minisig" -t "hacivat-spike $Version"
}
Write-Output "done: $Out"
