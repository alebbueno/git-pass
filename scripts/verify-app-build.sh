#!/usr/bin/env bash
# Verificação de release do GitPass: frontend (tsc + vite + sidecars), testes de contrato e Tauri build.
# Uso: ./scripts/verify-app-build.sh [--quick]
#   --quick  pula `tauri build` (útil em CI rápido ou smoke local).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
QUICK=false
for arg in "$@"; do
  case "$arg" in
    --quick) QUICK=true ;;
    -h | --help)
      echo "Uso: $0 [--quick]"
      exit 0
      ;;
    *)
      echo "Opção desconhecida: $arg" >&2
      exit 2
      ;;
  esac
done

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/Volumes/Dados/Server/git-pass-wt/target-build-gate}"
mkdir -p "$CARGO_TARGET_DIR"

log() { printf '\n== %s\n' "$*"; }

log "TypeScript (gitpass-desktop)"
(cd "$ROOT/gitpass-desktop" && npx tsc --noEmit)

log "Frontend + sidecars (npm run build:app)"
(cd "$ROOT/gitpass-desktop" && npm run build:app)

log "Testes de contrato (gitpass-desktop)"
(cd "$ROOT" && cargo test -p gitpass-desktop contract --quiet)

if [[ "$QUICK" == true ]]; then
  log "Modo --quick: tauri build omitido."
  log "Verificação concluída."
  exit 0
fi

log "Tauri build (GitPass.app + DMG)"
(cd "$ROOT/gitpass-desktop" && npm run tauri build)

log "Verificação concluída."
if [[ -d "$ROOT/target/release/bundle/macos/GitPass.app" ]]; then
  echo "App: $ROOT/target/release/bundle/macos/GitPass.app"
fi
