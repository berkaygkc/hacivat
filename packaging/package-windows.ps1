# Windows: UNSIGNED per-user NSIS installer. CI never signs (L13); release installers are
# minisigned afterwards on the PO's Mac with packaging/sign-artifacts.sh.
# usage: pwsh packaging/package-windows.ps1 <version>
param([Parameter(Mandatory = $true)][string]$Version)
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")
$Root = (Get-Location).Path
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root "target" }
$Out = Join-Path $TargetDir "packager/$Version"
if (Test-Path $Out) { Remove-Item -Recurse -Force $Out }
New-Item -ItemType Directory -Force $Out | Out-Null
cargo build --release --locked -p hacivat
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
Copy-Item (Join-Path $TargetDir "release/hacivat.exe") $Out
$Cfg = Join-Path $Root "packaging/.Packager.generated.toml"
(Get-Content packaging/Packager.toml) |
  Where-Object { $_ -notmatch "@SIGNING_IDENTITY@" } |
  ForEach-Object { $_ -replace "@VERSION@", $Version } |
  Set-Content $Cfg
try {
  cargo packager -c $Cfg --formats nsis --out-dir $Out
  if ($LASTEXITCODE -ne 0) { throw "cargo packager failed" }
} finally { Remove-Item -Force $Cfg -ErrorAction SilentlyContinue }
Write-Output "done (unsigned): $Out"
