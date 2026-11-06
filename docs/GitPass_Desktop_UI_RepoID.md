# GitPass desktop · identidade RepoID

O painel web (`gitpass-desktop`) segue o kit em [`RepoID_Kit_Visual`](./RepoID_Kit_Visual/00_LEIA-ME.md).

## O que está integrado

| Artefacto do kit | Uso na app |
| --- | --- |
| `Paleta/repoid-tokens.css` | Copiado para `gitpass-desktop/src/repoid-tokens.css` e importado no `styles.css`. Tema claro/escuro e tokens de estado (sucesso, aviso, erro) e perfis (iris, aqua, blue, rose). |
| `Logo/RepoID-simbolo.svg` | Marca na barra lateral (`src/assets/branding/`), inserido inline no HTML. |
| `Logo/RepoID-logo-fundo-claro.svg` / `…-escuro.svg` | Wordmarks por tema (altura ~22px). |
| Materiais (HIG) | **Vidro** na barra lateral e na faixa do topo (`backdrop-filter` + `var(--rid-material-tint)`), com **fallback** opaco se `prefers-reduced-transparency`. Listas e painéis usam **superfície opaca** (`--rid-surface`). |
| Tipografia | UI: `system-ui` / SF (kit). Código/commits: `ui-monospace` / SF Mono (`--rid-font-code`). |

## Tema

- **Auto**: segue `prefers-color-scheme` (atributo `data-theme` ausente no `<html>`).
- **Claro / Escuro**: `data-theme="light"` ou `dark"` no `<documentElement>`, persistido em `localStorage` (`gitpass-theme`).

A indigo (`#5545D6`) aciona ações no modo claro; a iris (`#B3A5FF`) no escuro, conforme o kit.

## Cartões por conta

Cada conta na grelha “Na cloud” recebe uma das quatro famílias de cor de perfil do token (`iris`, `aqua`, `blue`, `rose`) de forma **determinística** a partir do `id` da conta, para manter a mesma cor entre sessões.

## Build

**Regra:** sempre que o código do painel mudar (`src/`, `index.html`, assets), corre a build para atualizar `dist/` — evita abrir uma app com JS/CSS antigos.

```bash
cd gitpass-desktop && npm run build
```

Para o pacote Tauri: `npm run tauri build` (ou `tauri dev`).
