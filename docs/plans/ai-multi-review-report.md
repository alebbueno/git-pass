# ai-multi-review — resumo

**Veredito: GO condicional** para a Fase 0 (disposição da skill: APPROVED com condição).

- Data: 2026-11-08 · Worktree: `/Volumes/Dados/Server/git-pass-wt/ai-brainstorm` (`agent/ai-brainstorm` @ `fe7b50d`, igual a `main`)
- Relatório completo: `docs/plans/ai-llm-multi-review.md`
- Plano ajustado: `docs/plans/ai-llm.md` (ajustes marcados com AI-DL-028…041)
- Decisões 1–9: **intactas** (nenhuma `AI-DL-REV`). Nenhum código, commit ou push.

## Condição (bloqueador de processo)

**C1 — `main` não compila e não tem o padrão de Keychain que a Fase 0 copia.**
`cargo check -p gitpass-core` em `fe7b50d` falha: `lib.rs:51` reexporta `github_token_exists`, que só existe no `secrets.rs` não commitado do checkout principal (marcador `tokens/<id>.present` + `KEYCHAIN_IO`). O pré-requisito passa a gatear a Fase 0: integrar `secrets.rs`, `lib.rs`, `commit_review`, `identity_diagnostic`, `work_radar`, `pr_inbox` até `cargo check`/`cargo test` verdes.

Bloqueadores de design: **nenhum** depois dos ajustes já aplicados no plano.

## Principais objeções aceitas

| ID | Objeção | Resolução no plano |
|----|---------|--------------------|
| G-01 | “Daemon/CLI não linkam IA” não é garantido pelo build: `ai/` em `gitpass-core`, que `gitpassd` e CLI usam | Feature Cargo `ai` só no app; verificação com `cargo tree` (AI-DL-029) |
| S-02/G-02 | “Catálogo único” ambíguo: redação redige qualquer tamanho, detecção exige ≥ 36 e ignora fixtures; unificar ingenuamente regride logs ou o push | Lista única de tipos, padrão por uso; novas detecções como Warning; testes atuais sem edição (AI-DL-030) |
| S-03 | Endpoint OpenAI em aberto (“ou Responses”); `temperature` obrigatória quebra modelos de raciocínio | Chat Completions fixado; `temperature` opcional (AI-DL-031) |
| S-04/U-02 | “Testar conexão” valida autenticação, não saldo; 403 virava “chave inválida” | Textos 401/403 distintos; “saldo e limites não verificados”; sem retry em `insufficient_quota` (AI-DL-032) |
| S-07 | `send(id)` reutilizável + “Cancelar” com thread viva → cobrança dupla | ID de uso único; 1 envio em andamento (AI-DL-038) — Fase 1 |
| S-05/S-06 | “E-mails nunca saem” e “owner/repo omitidos” não eram garantidos para assuntos e caminhos | Redação de e-mails no pipeline de IA; omissão documentada como *best-effort* (AI-DL-036/037) — Fase 1 |
| U-01 | Fase 0 sozinha num release = tela de configuração sem uso | Fase 0 só sai junto com a Fase 1 (AI-DL-034) |

Também aceitas: contador de uso e limitador movidos para a Fase 1 (S-11), escopo sem upstream (S-09), hash e pré-visualização incluem o prompt de sistema (S-08), teto no store em memória (G-03), escrita atômica do `ai.toml` (G-06), textos de erro e de consentimento (U-03/04/05).

**Rejeitadas (5):** tirar `complete()` da Fase 0 (S-10); trava no `consent_version` (S-13, decisão 8); proxy/TLS corporativo (G-04, igual às chamadas GitHub — só documentado em GP-AI-006); confirmação menos frequente (U-06, AI-DL-011); mostrar parte da chave (U-07).

Total: 26 objeções em 24 itens → 19 aceitas, 5 rejeitadas, 0 pendentes.

## Próximo passo

1. Cumprir C1 (integração em `main` + build/testes verdes).
2. Usuário revisa os ajustes em `docs/plans/ai-llm.md`.
3. Plano de implementação da Fase 0: feature `ai` + `ai.toml` → Keychain → catálogo → adaptador OpenAI + teste de fio → erros → Configurações.

## Atenção

`scripts/run-ai-multi-review-agent.sh` copia `docs/plans/ai-llm.md` do checkout principal **por cima** do worktree. Copie os arquivos de `docs/plans/` do worktree para o checkout principal antes de rodar o script de novo, ou os ajustes se perdem.
