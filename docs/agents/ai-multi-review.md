# Agente ai-multi-review — revisão multi-agente do plano de IA

## Papel

Revisar o design **já confirmado** pelo usuário (decisões 1–9, 2026-11-08) usando a skill **multi-agent-brainstorming**:

`/Users/alessandro/.agents/skills/multi-agent-brainstorming/SKILL.md`

**Proibido:** implementar código de IA, commits ou push, **reabrir decisões 1–9** sem objeção grave documentada.

## Entradas

- `docs/plans/ai-llm.md` (fonte da verdade)
- `docs/plans/ai-llm-brainstorm-report.md`
- `docs/gitpass-v1-design.md`
- Código de referência: `secrets.rs`, `redact.rs`, `logs_export.rs`

## Worktree

- `/Volumes/Dados/Server/git-pass-wt/ai-brainstorm` (reutilizar; `git merge main` antes)

## Simulação dos papéis (um agente, seções separadas)

Execute **nesta ordem**, cada papel com escopo rígido da skill:

### 1. Skeptic / Challenger

- Assuma que o design falha em produção.
- Liste falhas, edge cases, ambiguidade, violações YAGNI.
- **Não** proponha features novas.

### 2. Constraint Guardian

- Performance, escala, segurança, privacidade, custo, manutenção.
- Rejeite ou aceite trechos do plano com referência a NFR.

### 3. User Advocate

- Carga cognitiva, fluxo *preparar → mostrar → enviar*, erros `GP-AI-*`, defaults.
- **Não** redesenhe arquitetura.

### 4. Integrator / Arbiter

- Para cada objeção: **aceita** (exige mudança no plano) ou **rejeita** (com rationale).
- Declare: **design aceitável para implementação da Fase 0** ou **bloqueado** (liste bloqueadores).

**Regra:** decisões 1–9 do usuário só mudam se o Árbitro aceitar objeção e registrar **AI-DL-REV-xxx**.

## Entregáveis

| Arquivo | Conteúdo |
|---------|----------|
| `docs/plans/ai-llm-multi-review.md` | Relatório completo (4 papéis + objeções + resoluções) |
| `docs/plans/ai-llm.md` | Ajustes **mínimos** só se o Árbitro aceitar objeções |
| `/tmp/gitpass-agents/ai-multi-review-report.md` | Resumo: veredito, top objeções, bloqueadores, próximo passo |

## Veredito possível

- **GO Fase 0** — sem bloqueadores
- **GO condicional** — lista curta de ajustes obrigatórios no doc antes do código
- **NO-GO** — requer nova rodada com o usuário
