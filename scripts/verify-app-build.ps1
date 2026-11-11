# Verificação de release do GitPass no Windows: frontend, testes de contrato e Tauri build (NSIS).
# Uso: .\scripts\verify-app-build.ps1 [-Quick]
param([switch]$Quick)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
if (-not $env:CARGO_TARGET_DIR) {
  $env:CARGO_TARGET_DIR = Join-Path $Root "target\ci-windows"
}
New-Item -ItemType Directory -Force -Path $env:CARGO_TARGET_DIR | Out-Null

function Log($msg) { Write-Host "`n== $msg" }

Log "TypeScript (gitpass-desktop)"
Push-Location (Join-Path $Root "gitpass-desktop")
try {
  npx tsc --noEmit
  Log "Frontend + sidecars (npm run build:app)"
  npm run build:app
} finally {
  Pop-Location
}

Log "Testes de contrato (gitpass-desktop)"
Push-Location $Root
try {
  cargo test -p gitpass-desktop contract --quiet
} finally {
  Pop-Location
}

if ($Quick) {
  Log "Modo -Quick: tauri build omitido."
  Log "Verificação concluída."
  exit 0
}

Log "Tauri build (NSIS)"
Push-Location (Join-Path $Root "gitpass-desktop")
try {
  npm run tauri build
} finally {
  Pop-Location
}

Log "Verificação concluída."
$nsis = Join-Path $env:CARGO_TARGET_DIR "release\bundle\nsis"
if (Test-Path $nsis) {
  Get-ChildItem $nsis -Filter "*.exe" | ForEach-Object { Write-Host "Instalador: $($_.FullName)" }
}
