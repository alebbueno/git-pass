# ai-brainstorm — resumo executivo

- **Commit base:** `fe7b50d4ef5655cd142a53e877492d0cb568d363` (branch `agent/ai-brainstorm`, igual a `main`)
- **Data:** 2026-11-08
- **Implementação:** nenhuma. Só documentação. Nada commitado, nada enviado ao remoto.
- **Status da skill:** *understanding lock* **confirmado** 2026-11-08 (decisões 1–9 com o usuário).

## Docs atualizados

- `/Volumes/Dados/Server/git-pass-wt/ai-brainstorm/docs/plans/ai-llm.md` — plano revisado (PT-BR)
- `/Volumes/Dados/Server/git-pass-wt/ai-brainstorm/docs/plans/ai-llm-brainstorm-report.md` — relatório completo (contexto, NFR, understanding lock, abordagens, design, modos de falha, decision log AI-DL-001…027)

## Principais mudanças vs. plano original

1. **Sem `ai_complete` genérico.** Comandos por feature com fluxo *preparar → mostrar → enviar*; o payload exibido é exatamente o enviado.
2. **IA só no processo do app.** Daemon, CLI e hooks ficam fora (preserva NFR de RAM e latência).
3. **`ai.toml` separado** do `config.toml`; sem provedor padrão embutido.
4. **Gemini autentica por header**, não `?key=`; erros de transporte sem URL (o `reqwest::Error` imprime a URL).
5. **Testar conexão via listagem de modelos** (sem custo), em vez de prompt “Olá”.
6. **Catálogo único de segredos** para detecção e redação; hoje `redact.rs` não cobre chaves de IA, AWS, Slack nem Stripe (que `outgoing.rs` detecta).
7. **Minimização:** sem e-mails, autores pseudonimizados, owner/repo omitidos por padrão.
8. **Saída como texto** (`textContent`), nunca `innerHTML`; alertas reais em bloco determinístico separado do texto da IA.
9. **Escopo enxugado:** removidos “Explicar GP-*”, “Ideias de próximo passo” e cache de respostas; “Resumir PR” vai para a Fase 3; sugestão de commit fica condicional e depois do novo **Fase 1.1 (diff opt-in)**.
10. Timeouts 10 s / 120 s, no máximo 1 retry e nunca após timeout; limite local de 20/hora; contador de tokens.
11. **Pré-requisito descoberto:** `commit_review.rs`, `work_radar.rs`, `pr_inbox.rs` e `identity_diagnostic.rs` só existem como trabalho não commitado no checkout principal.

## Top 5 riscos

1. Vazamento de segredo ou código ao provedor (redação é *best-effort*).
2. Vazamento da própria chave de IA (URL em erros, `?key=` do Gemini, `Debug`).
3. Abuso da chave ou XSS a partir da WebView (prompt livre; saída em `innerHTML`).
4. Relatório enganoso por prompt injection em commits de terceiros ou por alucinação.
5. Custo e deriva de provedores (modelos descontinuados, mudanças de API e de política de dados).

## Decisões que ainda precisam do usuário

1. Confirmar o *understanding lock* (relatório, seção 3).
2. ~~Três provedores na Fase 0 ou um só?~~ **✅ Usuário: A — só OpenAI na Fase 0.**
3. ~~Sugestão de mensagem de commit na v1?~~ **✅ Usuário: A — fora da v1.**
4. ~~Diff no relatório?~~ **✅ Usuário: A — sem diff no MVP; opt-in na 1.1.**
5. ~~Privacidade no payload~~ **✅ Usuário: A — pseudonimizar autores e omitir owner/repo por padrão.**
6. ~~Gemini plano gratuito?~~ **✅ Usuário: A — avisar, não bloquear.**
7. ~~Limites padrão?~~ **✅ Usuário: A — 20/h, 1 500 tokens, 48k chars.**
8. ~~Provedor local?~~ **✅ Usuário: A — só Fase 3.**
9. ~~MDM corporativo?~~ **✅ Usuário: A — não na v1.**
10. ~~Idioma~~ **✅ Usuário: A — pt-BR padrão.**
11. Quando integrar em `main` os módulos em andamento do checkout principal.
12. Commitar estes docs no branch `agent/ai-brainstorm`? (padrão: não, aguardando revisão)

## Próximo passo recomendado

Respostas do usuário → integrar o trabalho em andamento em `main` → revisão `multi-agent-brainstorming` → plano de implementação da Fase 0.
