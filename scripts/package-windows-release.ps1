#Requires -Version 5.1
<#
.SYNOPSIS
  Build release argus-local.exe and stage a portable Windows x64 zip + SHA256.

.DESCRIPTION
  ARG-33 packaging helper. Does NOT create a GitHub Release.
  See docs/RELEASE.md for the approved `gh release create` steps.
#>
param(
  [string]$OutDir = "proof/arg-33",
  [string]$DistDir = "dist",
  [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

if (-not $SkipBuild) {
  Write-Host "==> cargo build --release"
  cargo build --release
}

$exeSrc = "target/release/argus-local.exe"
if (-not (Test-Path $exeSrc)) {
  throw "Missing $exeSrc — run cargo build --release first"
}

New-Item -ItemType Directory -Force -Path $OutDir, $DistDir | Out-Null

$exeName = "argus-local.exe"
$zipName = "argus-local-windows-x64.zip"
$exeDst = Join-Path $OutDir $exeName
$zipDst = Join-Path $OutDir $zipName

Copy-Item $exeSrc $exeDst -Force
$exeHash = (Get-FileHash $exeDst -Algorithm SHA256).Hash.ToLower()
Set-Content -Path (Join-Path $OutDir "$exeName.sha256") -Value "$exeHash  $exeName" -NoNewline

if (Test-Path $zipDst) { Remove-Item $zipDst -Force }
Compress-Archive -Path $exeDst,(Join-Path $OutDir "$exeName.sha256") -DestinationPath $zipDst -Force

$zipHash = (Get-FileHash $zipDst -Algorithm SHA256).Hash.ToLower()
$zipShaPath = Join-Path $OutDir "$zipName.sha256"
Set-Content -Path $zipShaPath -Value "$zipHash  $zipName" -NoNewline

Copy-Item $zipDst (Join-Path $DistDir $zipName) -Force
Copy-Item $zipShaPath (Join-Path $DistDir "$zipName.sha256") -Force

Write-Host ""
Write-Host "EXE:  $exeDst"
Write-Host "      SHA256=$exeHash"
Write-Host "ZIP:  $zipDst"
Write-Host "      SHA256=$zipHash"
Write-Host "Also copied to $DistDir/"
Write-Host ""
Write-Host "Next (after approval): see docs/RELEASE.md for gh release create"