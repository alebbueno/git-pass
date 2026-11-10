# Agente ai-brainstorm — revisão do plano de IA (sem implementação)

## Papel

Melhorar **`docs/plans/ai-llm.md`** usando o processo da skill **brainstorming** (`/Users/alessandro/.agents/skills/brainstorming/SKILL.md`).

**Proibido nesta rodada:** implementar código, adicionar crates, comandos Tauri ou UI de IA. Apenas **design, decisões e documentação**.

## Worktree

- **Raiz:** `/Volumes/Dados/Server/git-pass-wt/ai-brainstorm`
- **Branch:** `agent/ai-brainstorm` (espelho de `main`; sincronize antes de trabalhar)
- **Não edite** `/Volumes/Dados/Server/git-pass` (checkout principal) salvo pedido explícito do usuário

### Sincronizar

```bash
cd /Volumes/Dados/Server/git-pass-wt/ai-brainstorm
git fetch origin main 2>/dev/null || true
git merge -q main 2>/dev/null || git merge -q origin/main 2>/dev/null || true
```

## Contexto do produto (leitura obrigatória)

1. `docs/plans/ai-llm.md` — plano atual
2. `docs/gitpass-v1-design.md` — NFR (telemetria zero, Keychain, hooks)
3. `crates/gitpass-core/src/secrets.rs` — padrão Keychain
4. `crates/gitpass-core/src/redact.rs` — redação
5. `crates/gitpass-core/src/commit_review.rs` — dado de commits já existente

## Processo (skill brainstorming)

Siga **na ordem**:

1. **Contexto** — o que já existe vs. proposto (notas curtas).
2. **NFR** — performance, privacidade, escala, custo, manutenção; marque suposições.
3. **Understanding lock** — resumo 5–7 bullets + suposições + perguntas abertas (como se fosse validar com o usuário; no relatório, proponha respostas padrão onde o usuário não estiver presente).
4. **2–3 abordagens** — ex.: só Rust vs. híbrido; um provedor vs. três; MVP só metadados vs. diff opt-in.
5. **Design incremental** — arquitetura, dados, erros, testes, fases revisadas.
6. **Decision log** — cada decisão, alternativas, motivo.

Opcional (se tempo): uma passagem **Skeptic** no espírito de `multi-agent-brainstorming` — seção “Modos de falha” no relatório.

## Entregáveis (arquivos a escrever no worktree)

| Arquivo | Conteúdo |
|---------|----------|
| `docs/plans/ai-llm-brainstorm-report.md` | Relatório completo do processo + decision log + riscos |
| `docs/plans/ai-llm.md` | **Atualizar** com melhorias acordadas no relatório (manter em PT-BR) |

## Relatório espelho (para o coordenador)

Além dos arquivos acima, escreva um resumo executivo em:

`/tmp/gitpass-agents/ai-brainstorm-report.md`

Incluir:

- Commit (`git rev-parse HEAD`)
- Principais mudanças sugeridas vs. plano original
- Top 5 riscos
- Decisões que **ainda precisam do usuário** (lista numerada)
- Caminho dos docs atualizados

## Git

- **Não** commite a menos que o relatório mande explicitamente e as mudanças sejam só em `docs/plans/*` e `docs/agents/*`.
- **Não** push.

## Encerramento

Pare após documentar. Não iniciar Fase 0 de implementação.
