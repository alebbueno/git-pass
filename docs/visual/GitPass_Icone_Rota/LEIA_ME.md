# GitPass · Ícone Rota

O ícone do aplicativo usa a marca da Splash Rota. O PNG mestre de 1024 × 1024 px tem transparência ao redor da base de vidro.

## Aplicar no projeto

1. Na raiz do repositório: `./scripts/regen-macos-icons.sh` (usa `icon.png` desta pasta; recria `icon.icns` e os PNGs do Tauri).
2. Gere um novo build no macOS. A partir de `gitpass-desktop/`, rode `npm run tauri build`.
3. Abra o aplicativo gerado no novo build para conferir o ícone no Finder e no Dock.

Se `GitPass_Rota_1024.png` estiver truncado no disco, regenere o mestre a partir de `icon.png` ou rode `npx tauri icon docs/visual/GitPass_Icone_Rota/icon.png -o src-tauri/icons`.

Os nomes e tamanhos dos arquivos correspondem ao checkout de referência `49cecf9`. O `icon.icns` inclui os mesmos onze formatos do arquivo do projeto. `GitPass_Rota_1024.png` é o mestre para futuros exports.

## Direção da arte

Arte produzida pela geração integrada de imagens com `Splash_Rota.png` como referência visual.

```text
Create a single production-quality macOS Dock application icon for GitPass, a square PNG with real transparent background. The reference is the app splash art. Extract the design language and preserve the EXACT central Rota brand glyph geometry as a faithful shape, while redesigning its tile to look premium at Dock scale. The glyph has ONLY three nodes: solid lavender lower-left dot, hollow pale-lavender upper-left circle, solid mint right endpoint. The route starts at the lower-left dot, travels straight horizontally right, curves then rises diagonally to the right mint endpoint; at the junction a secondary arm rises straight vertically then curves left into the upper-left hollow circle. No additional stems, no holes, no added nodes. This same glyph is the hero, smooth thick optical stroke in pale luminous lavender. Center it precisely on a single deep ink navy rounded-square macOS-style tile, front-facing orthographic, generous 20% corner radius, sharp silhouette, tile occupies approximately 81% of square canvas width and height, balanced transparent margins on all sides. Subtle premium material: dark smoked glass, a light-catching thin violet rim along the upper-left and muted teal rim lower-right, restrained violet-to-petrol gradient internally, controlled specular highlights just along the top edge. The icon should feel like a dense, elegant hardware inlay. The mark occupies about 59% of the tile width and has high contrast at 32 px. Outside the tile must be entirely transparent, with no backdrop, no pedestal, no cast shadow, no orbital lines outside the tile. No title, letters, words, text, watermark or other icons. Do not retain the rest of the splash scene. Output one isolated perfectly centered icon.
```
