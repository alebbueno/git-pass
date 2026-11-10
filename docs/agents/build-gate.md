# Agente build-gate — verificação de build após mudanças no app

## Papel

Garantir que **cada atualização** no app desktop (UI, Tauri, crates usados pelo app) ainda **compila, passa nos testes de contrato e gera o `.app`**.

Não implementa features. Só verifica, corrige **apenas** quebras óbvias de build (imports, typos) se forem triviais e estiverem na área que falhou; caso contrário documenta no relatório.

## Worktree

- **Raiz:** `/Volumes/Dados/Server/git-pass-wt/build-gate`
- **Branch:** `agent/build-gate` (espelho de `main`; sincronize antes de verificar)
- **Cargo:** `export CARGO_TARGET_DIR=/Volumes/Dados/Server/git-pass-wt/target-build-gate`
- **Não edite** o checkout principal em `/Volumes/Dados/Server/git-pass` enquanto outro agente trabalha lá; use só o worktree.

### Sincronizar com `main`

```bash
cd /Volumes/Dados/Server/git-pass-wt/build-gate
git fetch origin main 2>/dev/null || true
git merge -q origin/main 2>/dev/null || git merge -q main
```

Se houver conflito, pare e reporte (não resolva conflitos de feature).

## Quando rodar

Rode este agente **sempre** que houver commits em `main` (ou no branch atual) que toquem:

- `gitpass-desktop/src/**`, `gitpass-desktop/index.html`, `gitpass-desktop/src-tauri/**`
- `crates/**` usados pelo desktop (`gitpass-core`, CLI embutido, daemon sidecar)
- `scripts/verify-app-build.sh`, `scripts/build-sidecars.mjs`
- Ícones em `gitpass-desktop/src-tauri/icons/**`

Pule se o diff for só documentação fora desses caminhos.

## Verificação (obrigatória)

Na raiz do worktree:

```bash
chmod +x scripts/verify-app-build.sh
./scripts/verify-app-build.sh
```

O script roda, nesta ordem:

1. `npx tsc --noEmit`
2. `npm run build:app` (sidecars + Vite)
3. `cargo test -p gitpass-desktop contract`
4. `npm run tauri build` → `target/release/bundle/macos/GitPass.app`

Se falhar, capture o trecho relevante do log, tente **uma** correção mínima se for clara, e rode de novo **uma vez**. Se ainda falhar, documente no relatório sem commitar WIP quebrado.

## Git

- **Não** commite `gitpass-desktop/dist/`, `target/` nem `node_modules` (estão no `.gitignore`).
- Commite só se você corrigiu um erro de build: mensagem `fix(build): …`
- **Não** faça push salvo pedido explícita.

## Relatório

Escreva em `/tmp/gitpass-agents/build-gate-report.md`:

- Commit verificado (`git rev-parse HEAD`)
- Arquivos que motivaram a rodada (resumo do `git diff` ou lista de commits desde o último OK)
- Resultado de cada etapa (ok / falhou)
- Caminho do `.app` se build ok
- Duração aproximada
- O que não foi possível verificar (ex.: teste manual da janela)

## Disparo automático (local)

O agente **não roda sozinho** até você instalar o hook ou chamar o script manualmente.

```bash
./scripts/install-build-gate-hook.sh
```

Depois disso, cada `git commit` que mexer em `gitpass-desktop/`, `crates/`, etc. dispara `run-build-gate-agent.sh` em segundo plano.

## Lançar o agente (manual / coordenador)

```bash
/Volumes/Dados/Server/git-pass/scripts/run-build-gate-agent.sh
```

Ou manualmente:

```bash
~/.local/bin/cursor-agent -p --force --trust \
  --workspace /Volumes/Dados/Server/git-pass-wt/build-gate \
  --model claude-opus-5-5-high \
  "Leia docs/agents/build-gate.md, sincronize com main, rode ./scripts/verify-app-build.sh, escreva o relatório em /tmp/gitpass-agents/build-gate-report.md e encerre."
```
