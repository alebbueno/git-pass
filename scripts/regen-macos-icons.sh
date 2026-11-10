#!/usr/bin/env bash
# Regenera icon.icns e PNGs do Tauri a partir do mestre Rota (Finder lista usa 16/32 px do .icns).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# O mestre 1024 px pode corromper no disco; o icon.png do kit (512) é a referência estável.
SOURCE="${1:-$ROOT/docs/visual/GitPass_Icone_Rota/icon.png}"
MASTER_1024="$ROOT/docs/visual/GitPass_Icone_Rota/GitPass_Rota_1024.png"
ICONS="$ROOT/gitpass-desktop/src-tauri/icons"
ICONSET="$(mktemp -d -t gitpass-iconset)"

if [[ ! -f "$SOURCE" ]]; then
  echo "Mestre não encontrado: $SOURCE" >&2
  exit 1
fi

cleanup() { rm -rf "$ICONSET"; }
trap cleanup EXIT

mk() {
  sips -z "$2" "$3" "$SOURCE" --out "$ICONSET/$1" >/dev/null
}

mk icon_16x16.png 16 16
mk icon_16x16@2x.png 32 32
mk icon_32x32.png 32 32
mk icon_32x32@2x.png 64 64
mk icon_128x128.png 128 128
mk icon_128x128@2x.png 256 256
mk icon_256x256.png 256 256
mk icon_256x256@2x.png 512 512
mk icon_512x512.png 512 512
sips -z 1024 1024 "$SOURCE" --out "$ICONSET/icon_512x512@2x.png" >/dev/null

xattr -cr "$ICONSET" 2>/dev/null || true
if ! iconutil -c icns "$ICONSET" -o "$ICONS/icon.icns" 2>/dev/null; then
  echo "iconutil falhou; usando \`tauri icon\`…" >&2
  (cd "$ROOT/gitpass-desktop" && npx tauri icon "$SOURCE" -o src-tauri/icons >/dev/null)
fi
cp "$ICONSET/icon_32x32.png" "$ICONS/32x32.png"
cp "$ICONSET/icon_32x32@2x.png" "$ICONS/64x64.png"
cp "$ICONSET/icon_128x128.png" "$ICONS/128x128.png"
cp "$ICONSET/icon_128x128@2x.png" "$ICONS/128x128@2x.png"
cp "$ICONSET/icon_512x512.png" "$ICONS/icon.png"
sips -z 1024 1024 "$SOURCE" --out "$MASTER_1024" >/dev/null 2>&1 || true
cp "$ICONSET/icon_512x512@2x.png" "$ROOT/gitpass-desktop/src/assets/app-icon.png" 2>/dev/null || true

echo "Ícones atualizados em $ICONS (fonte: $SOURCE)"
