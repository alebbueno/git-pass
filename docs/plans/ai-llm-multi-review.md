# Revisão multi-agente — IA com chave do usuário (BYOK)

| Campo | Valor |
|-------|-------|
| Documento revisado | `docs/plans/ai-llm.md` (design com understanding lock e decisões 1–9 confirmados em 2026-11-08) |
| Processo | Skill `multi-agent-brainstorming`, fase 2 (Skeptic → Constraint Guardian → User Advocate) e fase 3 (Integrator/Arbiter), simulados por um agente em seções separadas |
| Entradas | `ai-llm.md`, `ai-llm-brainstorm-report.md`, `gitpass-v1-design.md`, `secrets.rs`, `redact.rs`, `logs_export.rs`, `outgoing.rs`, `commit_review.rs` (checkout principal), `Cargo.toml`, `tauri.conf.json` |
| Branch / base | `agent/ai-brainstorm` @ `fe7b50d` (`git merge main`: já atualizado) |
| Data | 2026-11-08 |
| Implementação | **Nenhuma.** Nenhum código, commit ou push. Só documentos. |
| Decisões 1–9 | **Nenhuma reaberta.** Nenhuma objeção exigiu `AI-DL-REV-xxx`. |

---

## 0. Verificação factual (antes dos papéis)

Premissas do plano conferidas no código. As marcadas ❌ sustentam objeções abaixo.

| # | Premissa do plano | Resultado | Evidência |
|---|-------------------|-----------|-----------|
| V1 | `main` é base sólida; o pré-requisito só afeta a Fase 1 | ❌ | `cargo check -p gitpass-core` em `fe7b50d` falha: `lib.rs:51` reexporta `github_token_exists`, que não existe no `secrets.rs` commitado. |
| V2 | Keychain com “arquivo marcador + cache + mutex, como em `secrets.rs`” | ❌ em `main` | O `secrets.rs` de `main` só tem `TOKEN_CACHE`. Marcador (`tokens/<id>.present`), `KEYCHAIN_IO` e `github_token_exists` existem **apenas** no trabalho não commitado do checkout principal (`/Volumes/Dados/Server/git-pass`). |
| V3 | `gitpassd`, CLI e hooks “não linkam IA” | ⚠️ não imposto | O plano põe a IA em `gitpass-core/src/ai/`; `gitpassd` e `gitpass-cli` dependem de `gitpass-core` sem features. Nada impede um uso acidental no daemon. |
| V4 | Catálogo único de segredos para detectar e redigir | ⚠️ semântica divergente | `redact.rs` redige `gh[opsur]_` de **qualquer tamanho**; `outgoing::secret_kind` só detecta com **≥ 36** caracteres, ignora linhas de teste/fixture (`added_line_is_secret_fixture`) e gera `SecretSuspected` com severidade **Warning** no push. |
| V5 | Export de logs: “manifesto informa chave configurada sim/não” | ⚠️ | `Manifest` (`logs_export.rs`) tem só versões e arquivos; o lugar natural é um item em `diagnostics.json`. |
| V6 | `commit_review` fornece escopo “não enviados” | ⚠️ | `beyond_upstream_shas` devolve `None` sem upstream → `reach = Unknown`. Branch nova nunca enviada não tem escopo “não enviados” definido. |
| V7 | `reqwest` blocking já no projeto | ✅ | `reqwest 0.12.28`, `default-features = false`, `blocking/json/rustls-tls`. Consequência: sem `system-proxy` e com raízes `webpki` (não as do Keychain). |
| V8 | CSP sem hosts externos | ✅ | `connect-src: ipc: http://ipc.localhost` (produção). |
| V9 | `tiny_http` disponível para testes de fio | ✅ | Dependência normal do `gitpass-core`. |
| V10 | `config.toml` descarta campos desconhecidos ao regravar | ✅ | `AppConfig` sem `flatten`/`deny_unknown_fields`; justifica `ai.toml` separado. |
| V11 | Saída via `textContent` é necessária | ✅ | 115 usos de `innerHTML` no frontend. |

---

## 1. Skeptic / Challenger

> “Assuma que este design falha em produção. Por quê?” Sem propor features nem arquiteturas alternativas.

| ID | Objeção | Referência | Fase |
|----|---------|------------|------|
| S-01 | **O pré-requisito está subestimado.** O plano diz que só a Fase 1 depende do trabalho em andamento, mas `main` nem compila (V1) e o padrão de Keychain que a Fase 0 manda copiar não está em `main` (V2). Quem começar a Fase 0 sobre `main` vai reimplementar ou copiar código não revisado. | Pré-requisito; 0.2; AI-DL-027 | 0 |
| S-02 | **“Catálogo único” é ambíguo.** Unificar ingenuamente com um limiar só ou reduz a redação (hoje qualquer `gho_x` é redigido) ou dispara avisos novos no push (hoje só ≥ 36). As exclusões de fixture valem só para detecção. | 0.4; AI-DL-010 | 0 |
| S-03 | **Contrato do adaptador OpenAI indefinido.** “Chat Completions (ou Responses)” deixa as fixtures sem alvo. `AiRequest.temperature` é obrigatório, mas modelos de raciocínio da OpenAI rejeitam `temperature` diferente do padrão e usam parâmetro próprio de teto de saída; tokens de raciocínio consomem o teto de 1 500 e podem gerar resposta vazia com `finish = Truncated`. (Comportamento a confirmar na documentação da OpenAI na data da implementação.) | 0.3; decisão 6 | 0 (contrato), 1 (comportamento) |
| S-04 | **“Testar conexão” prova menos do que parece.** Listar modelos valida autenticação, não saldo: chave de conta sem crédito passa no teste e falha no primeiro relatório (429 `insufficient_quota`). Chave restrita pode receber 403 na listagem e funcionar em completions, mas seria rotulada “inválida ou revogada”. | 0.3; GP-AI-003/004; AI-DL-006 | 0 |
| S-05 | **“E-mails nunca saem” não é garantido.** O assunto do commit e os caminhos são texto livre e podem conter e-mails; o catálogo não redige e-mails. | Princípio “Minimização”; Fase 1 passo 3–4 | 1 |
| S-06 | **`include_repo_name = false` é *best-effort*.** Assuntos de merge (“Merge pull request #12 from owner/branch”), SHAs de repositório público e caminhos com nome de cliente identificam o repositório. O consentimento não pode prometer omissão total. | Decisão 4; AI-DL-009 | 1 |
| S-07 | **Envio duplicado e cobrança dupla.** Não está dito que `ai_request_send(id)` é de uso único. “Cancelar” libera a UI enquanto a thread bloqueante segue até 120 s; um novo envio roda em paralelo. | Fase 1 comandos/UX; AI-DL-016 | 1 |
| S-08 | **“O que você vê é o que sai” é ambíguo.** O `payloadText` exibido não inclui o prompt de sistema, o modelo nem os parâmetros que também saem; não está dito de que é o `sha256`. | Princípio; `ai_report_prepare` | 1 |
| S-09 | **Escopo “não enviados” sem upstream** cai em `reach = Unknown` (V6). Além disso, `commit_review` e o `git log --name-status` são duas leituras e podem ver `HEAD` diferentes se o usuário commitar no meio. | Fase 1 passos 1–2 | 1 |
| S-10 | **YAGNI:** `complete()` e as fixtures de completion entram na Fase 0 sem nenhum chamador. | 0.3; 0.7 | 0 |
| S-11 | **YAGNI:** contador de uso do mês e limitador local na Fase 0 não têm o que contar — a única chamada da Fase 0 (listar modelos) é gratuita. | 0.6; NFR custo | 0 |
| S-12 | **Marcador dessincronizado.** Se o usuário apagar o item no Acesso às Chaves, o marcador continua e o status mostra “chave configurada”. | 0.2 | 0 |
| S-13 | `consent_version` fica num arquivo editável; o consentimento pode ser “pulado”. | 0.1 | 0 |

---

## 2. Constraint Guardian

> Performance, escala, confiabilidade, segurança/privacidade, manutenção, custo. Sem debater objetivos de produto.

| ID | Objeção | NFR | Fase |
|----|---------|-----|------|
| G-01 | **A fronteira de processo não é imposta pelo build** (V3). O princípio “`gitpassd`, CLI e hooks não linkam IA” e o NFR de < ~50 MB do daemon (DL-012) dependem de disciplina, não do compilador. **Rejeito** o trecho “`gitpass-core/src/ai/`” sem mecanismo de isolamento. | Fronteira de processo; DL-012 | 0 |
| G-02 | **O catálogo mexe em caminho não-IA.** Alterar `redact`/`secret_kind` muda logs do daemon e avisos do push. O NFR “falha de IA nunca afeta validação, push ou hooks” exige que essa mudança seja isolada, com os testes atuais intactos. (Complementa S-02.) | Confiabilidade | 0 |
| G-03 | **Memória sem teto.** O store de pedidos preparados só tem TTL; uma UI com bug que chame `prepare` em loop acumula payloads de até 120k caracteres. | Confiabilidade/escala | 1 |
| G-04 | **Rede corporativa.** Com `default-features = false`, o `reqwest` 0.12.28 fica sem `system-proxy`, e `rustls-tls` usa raízes `webpki`, não as do Keychain: proxy do sistema e inspeção TLS corporativa falham. | Confiabilidade | 0 |
| G-05 | **Retry inútil em 429 sem crédito.** `insufficient_quota` também vem como 429; repetir só adia o erro (até 10 s) sem chance de sucesso. | Custo/latência | 0 |
| G-06 | **Escrita de `ai.toml`** precisa ser atômica (tmp + `rename`, como `config.rs`) para não corromper preferências e consentimento num crash. | Confiabilidade | 0 |

Trechos **aceitos** sem objeção: timeouts 10 s/120 s; máx. 1 retry e nunca após timeout; `without_url()`; Gemini por header; CSP inalterada; montagem de contexto com uma chamada `git log` (p95 < 1,5 s é folgado); redação por campo (≈ 100 × 21 campos × ~12 regex, custo desprezível); `tiny_http` sem dependência nova.

---

## 3. User Advocate

> Carga cognitiva, fluxo *preparar → mostrar → enviar*, erros `GP-AI-*`, padrões. Sem redesenhar arquitetura.

| ID | Objeção | Fase |
|----|---------|------|
| U-01 | **Fase 0 sozinha num release confunde.** O usuário ativa a IA, lê o consentimento, salva a chave… e não existe nada para usar. | 0 |
| U-02 | **Texto do resultado do teste** (ligado a S-04): “Chave válida” induz a achar que o relatório vai funcionar. | 0 |
| U-03 | **Desligar a IA não remove a chave**, e o usuário pode supor o contrário. | 0 |
| U-04 | **Consentimento e provedores futuros.** Se o texto da Fase 0 citar Anthropic/Gemini, fala de algo que não existe; se citar só OpenAI, a Fase 0.1 cria destinatários novos sem novo aceite. | 0 / 0.1 |
| U-05 | **Erros com ação ambígua.** GP-AI-004 mistura “sem crédito” (ação: billing) e “muitas requisições” (ação: aguardar). GP-AI-011 diz “Reportar”, mas não há telemetria: reportar como? | 0 |
| U-06 | Confirmação com payload **a cada chamada** cansa. | 1 |
| U-07 | Depois de salvar, não há como saber **qual** chave está salva (nem os últimos dígitos). | 0 |

---

## 4. Respostas do Primary Designer

| ID | Resposta / revisão proposta |
|----|-----------------------------|
| S-01 | Pré-requisito passa a **gatear a Fase 0**: integrar em `main` o `secrets.rs` (marcador, `KEYCHAIN_IO`, `github_token_exists`), `lib.rs` e os módulos em andamento; critério objetivo: `cargo check` e `cargo test` verdes em `main`. |
| S-02 / G-02 | Definir “catálogo único” como **uma lista de tipos com padrão por uso**: redação continua agressiva (qualquer tamanho), detecção mantém limiares e exclusões de fixture; novas detecções entram como Warning (igual hoje). Mudança em commit próprio; testes atuais de `redact` e `outgoing` passam sem edição. |
| S-03 | Fixar **Chat Completions** na Fase 0 (também é o protocolo dos provedores locais da Fase 3, decisão 7). `temperature` vira opcional (omitida quando o modelo não aceita). O teto de saída usa o parâmetro aceito pelo endpoint. Na Fase 1, `Truncated` com texto vazio → GP-AI-009 com dica; o modelo “recomendado” precisa caber nos 1 500 tokens (checklist manual). Não altera a decisão 6. |
| S-04 / U-02 | 401 → GP-AI-003 “inválida ou revogada”; 403 → GP-AI-003 com texto “sem permissão”. Texto do teste: “Chave reconhecida pelo provedor. Saldo e limites não são verificados.” |
| S-05 | Na montagem de contexto da IA, redigir e-mails em assuntos e caminhos (regra só do pipeline de IA, fora do catálogo global, para não mexer no push). Teste de fio já exige “nenhum e-mail”. |
| S-06 | Mantida a decisão 4. Documentar no plano e no consentimento que a omissão do nome vale para os campos estruturados; o texto dos commits pode identificar o repositório, e a pré-visualização é a garantia. |
| S-07 | ID consumido no `send` (segundo envio → GP-AI-013). No máximo 1 envio em andamento; “Cancelar” esconde o resultado, mas novo envio só depois que a thread anterior terminar. |
| S-08 | A pré-visualização mostra prompt de sistema (versão), modelo e teto de saída além dos dados; o `sha256` cobre o corpo JSON serializado enviado, sem headers. |
| S-09 | Sem upstream: “não enviados” fica indisponível com explicação e o escopo “últimos N” é oferecido. Os caminhos vêm do mesmo conjunto de SHAs já lido (`git log --name-status -z --no-walk <shas>`). |
| S-10 | Manter. O adaptador e o teste de fio são onde mora o risco de vazar a chave; validá-los antes de existir dado de repositório é o objetivo da Fase 0. |
| S-11 | Aceito: contador de uso e limitador entram na Fase 1, junto do primeiro chamador de `complete()`. `[limits]` continua no schema do `ai.toml` desde a Fase 0. |
| S-12 | Seguir o padrão do `secrets.rs` em andamento: `NoEntry` na leitura apaga o marcador e o cache; o envio responde GP-AI-002. |
| S-13 | Manter: decisão 8 (sem trava corporativa); quem edita o arquivo é o próprio usuário que consente. |
| G-01 | Módulo `ai` atrás da feature Cargo `ai` do `gitpass-core`, desligada por padrão e ligada só por `gitpass-desktop`. O catálogo de segredos fica **fora** da feature (o push também usa). Verificação: `cargo tree -e features -p gitpassd` e `-p gitpass-cli` sem `ai`. |
| G-03 | Store com teto pequeno de pedidos (valor no plano de implementação); acima dele, descarta o mais antigo. |
| G-04 | Manter como está na v1: mesmo comportamento das chamadas ao GitHub; mudar features do `reqwest` afeta o core inteiro. Documentar no detalhe de GP-AI-006. |
| G-05 | 429 com `insufficient_quota` → GP-AI-004 direto, sem retry. |
| G-06 | Escrita atômica do `ai.toml`, como `config.rs`. |
| U-01 | Fase 0 pode ir para `main`, mas **não vai sozinha para release de usuário**: publica junto com a Fase 1. |
| U-03 | Ao desligar: “A chave continua no Keychain. Use Remover chave para apagá-la.” |
| U-04 | Consentimento genérico (“o provedor da chave que você salvar”) + aviso específico ao salvar a chave de cada provedor (para Google, o checkbox da decisão 5). |
| U-05 | GP-AI-004 com dois textos (sem crédito → billing; limite de requisições → aguardar). GP-AI-011: “Exporte os logs (Ajuda → Exportar logs) e abra uma issue.” |
| U-06 | Manter: AI-DL-011 — cada chamada tem custo e o payload pode mudar. |
| U-07 | Manter: nenhum comando devolve parte da chave (§5.2 do relatório de brainstorming). Mostrar a data de gravação seria polish futuro, não requisito. |

---

## 5. Integrator / Arbiter

Critério: aceitar quando a objeção aponta ambiguidade, contradição com princípio/NFR já declarado ou fato verificado no código; rejeitar quando pede feature nova, reabre decisão sem evidência nova ou duplica comportamento existente.

| ID | Decisão | Rationale | Mudança no plano | Bloqueia Fase 0? |
|----|---------|-----------|------------------|------------------|
| S-01 | **Aceita** | Fato verificado (V1, V2). | Pré-requisito reescrito | **Sim — processo** (exige integração em `main`, não só doc) |
| S-02 + G-02 | **Aceita** | Ambiguidade que mexe em caminho não-IA (V4). | §0.4 | Sim — doc (aplicado) |
| S-03 | **Aceita** | Contrato precisa de alvo único para as fixtures. | §0.3 | Sim — doc (aplicado) |
| S-04 + U-02 | **Aceita** | Mensagem enganosa contra fato do provedor. | §0.3, §0.5, §0.6 | Não (aplicado) |
| S-05 | **Aceita** | Princípio declarado não era cumprido pelo pipeline. | Fase 1 passo 4 | Não (Fase 1) |
| S-06 | **Aceita (só texto)** | Evita promessa falsa; decisão 4 intacta. | Fase 1 passo 3 | Não (Fase 1) |
| S-07 | **Aceita** | Risco de custo concreto. | Fase 1 comandos/UX | Não (Fase 1) |
| S-08 | **Aceita** | Torna o princípio verificável. | Fase 1 comandos | Não (Fase 1) |
| S-09 | **Aceita** | Fato verificado (V6). | Fase 1 passos 1–2 | Não (Fase 1) |
| S-10 | **Rejeitada** | O adaptador é o ponto de risco da chave; testá-lo cedo é o objetivo da Fase 0. | — | — |
| S-11 | **Aceita** | YAGNI válido; valores da decisão 6 inalterados. | §0.6, Fase 1 | Não (aplicado) |
| S-12 | **Aceita (nota)** | Já resolvido pelo padrão em andamento; só precisa constar. | §0.2 | Não (aplicado) |
| S-13 | **Rejeitada** | Decisão 8; sem evidência nova. | — | — |
| G-01 | **Aceita** | Princípio não verificável sem mecanismo de build (V3). | Princípio “Só no app”, §0.3 | Sim — doc (aplicado) |
| G-03 | **Aceita** | Teto barato, evita crescimento sem limite. | Fase 1 comandos | Não (Fase 1) |
| G-04 | **Rejeitada (documentar)** | Mesmo comportamento das chamadas GitHub; mudar exige decisão do core inteiro, fora do escopo de IA. | §0.5 (nota GP-AI-006) | — |
| G-05 | **Aceita** | Retry sem chance de sucesso. | §0.3 | Não (aplicado) |
| G-06 | **Aceita** | Mesmo padrão já usado em `config.rs`. | §0.1 | Não (aplicado) |
| U-01 | **Aceita** | Descompasso entre intenção e experiência; não muda escopo de fase. | Entregável da Fase 0 | Não (aplicado) |
| U-03 | **Aceita** | Texto, sem mudança de comportamento. | §0.6 | Não (aplicado) |
| U-04 | **Aceita** | Coerente com “novo destinatário = aviso” e decisão 5. | §0.6 | Não (aplicado) |
| U-05 | **Aceita** | Ação sugerida precisa ser executável sem telemetria. | §0.5 | Não (aplicado) |
| U-06 | **Rejeitada** | AI-DL-011; custo por chamada. | — | — |
| U-07 | **Rejeitada** | Decisão de segurança explícita; o pedido é feature nova. | — | — |

**Contagem:** 26 objeções (13 Skeptic, 6 Guardian, 7 User Advocate), julgadas em 24 itens (S-02/G-02 e S-04/U-02 juntas) → **19 aceitas**, **5 rejeitadas** (S-10, S-13, G-04 com nota de documentação, U-06, U-07). Nenhuma pendente.

### Declaração do Árbitro

**Design aceitável para implementação da Fase 0**, com uma condição de processo:

- **C1 (bloqueador de processo):** `main` precisa compilar e conter o padrão de Keychain com marcador (`secrets.rs`/`lib.rs` em andamento) **antes** do primeiro commit da Fase 0. Não é falha de design, mas sem isso a Fase 0 não tem base.

Os ajustes de documento obrigatórios (S-02/G-02, S-03, G-01) e os demais aceitos **já foram aplicados** em `docs/plans/ai-llm.md` nesta rodada. Os itens aceitos da Fase 1 (S-05 a S-09, G-03) entram no plano agora e só bloqueiam o início da Fase 1.

**Veredito: GO condicional** — condição única: C1.
**Disposição (skill):** APPROVED com condição (C1).

---

## 6. Decision log (acréscimos desta revisão)

Continua a numeração de `ai-llm-brainstorm-report.md` §7. Nenhuma entrada `AI-DL-REV-xxx`: decisões 1–9 intactas.

| ID | Decisão | Alternativas consideradas | Objeções | Resolução / rationale |
|----|---------|---------------------------|----------|-----------------------|
| AI-DL-028 | Pré-requisito gateia a **Fase 0**; inclui `secrets.rs`/`lib.rs` em andamento; critério `cargo check` + `cargo test` verdes em `main` | Começar a Fase 0 sobre `main` atual; reimplementar o marcador no branch de IA | S-01 | `main` não compila; duplicar o padrão gera conflito |
| AI-DL-029 | Módulo `ai` atrás da feature Cargo `ai` (desligada por padrão, ligada só pelo app); catálogo de segredos fora da feature | Crate separado `gitpass-ai`; confiar em *dead code elimination* | G-01 | Feature é o mecanismo mínimo e verificável (`cargo tree`); crate novo é mais estrutura que o necessário agora |
| AI-DL-030 | Catálogo único = lista de tipos com padrão por uso (redação agressiva, detecção com limiar e exclusões); novas detecções como Warning; commit isolado | Um padrão só para tudo; manter duas listas | S-02, G-02 | Unificação sem regressão em redação nem em push |
| AI-DL-031 | OpenAI via **Chat Completions**; `temperature` opcional; teto de saída no parâmetro aceito pelo endpoint | Responses API; deixar em aberto | S-03 | Alvo único para fixtures; mesmo protocolo dos provedores locais da Fase 3 |
| AI-DL-032 | 401/403 com textos distintos; teste de chave não promete saldo; 429 `insufficient_quota` sem retry | Tratar 4xx igual | S-04, U-02, G-05 | Mensagens corretas e sem espera inútil |
| AI-DL-033 | Contador de uso e limitador local entram na Fase 1 | Fase 0 | S-11 | Nada para contar na Fase 0 |
| AI-DL-034 | Fase 0 não é publicada sozinha para usuários | Release só da Fase 0 | U-01 | Evita tela de configuração sem uso |
| AI-DL-035 | Consentimento genérico de destinatário + aviso específico ao salvar chave de cada provedor | Texto listando os 3 provedores | U-04 | Coerente entre Fase 0 e 0.1; decisão 5 intacta |
| AI-DL-036 | E-mails redigidos no pipeline de contexto da IA (fora do catálogo global) | Só documentar o risco; pôr e-mail no catálogo global | S-05 | Cumpre o princípio sem alterar avisos de push |
| AI-DL-037 | Omissão de owner/repo documentada como *best-effort* | Filtrar commits de merge | S-06 | Filtrar seria feature nova; pré-visualização é a garantia |
| AI-DL-038 | Pedido preparado de uso único; 1 envio em andamento; store com teto | Permitir reenvio; só TTL | S-07, G-03 | Evita cobrança dupla e crescimento de memória |
| AI-DL-039 | Pré-visualização inclui prompt de sistema, modelo e teto; hash do corpo JSON enviado | Hash só dos dados | S-08 | “O que você vê é o que sai” verificável |
| AI-DL-040 | Sem upstream: escopo “não enviados” indisponível com explicação; caminhos lidos dos mesmos SHAs | Tratar `Unknown` como “não enviado” | S-09 | Evita escopo errado e leitura inconsistente |
| AI-DL-041 | Proxy do sistema/inspeção TLS fora da v1, documentado em GP-AI-006 | Ligar `system-proxy`/raízes nativas | G-04 | Decisão do core inteiro, não da IA |

---

## 7. Critérios de saída da skill

| Critério | Status |
|----------|--------|
| Understanding lock completo | ✅ Confirmado pelo usuário em 2026-11-08 |
| Todos os revisores invocados | ✅ Skeptic, Constraint Guardian, User Advocate, Arbiter |
| Todas as objeções resolvidas ou rejeitadas explicitamente | ✅ 19 aceitas, 5 rejeitadas, 0 pendentes |
| Decision log completo | ✅ AI-DL-001…027 (brainstorming) + AI-DL-028…041 (esta revisão) |
| Árbitro declarou o design aceitável | ✅ Fase 0, condicionado a C1 |

## 8. Próximo passo

1. **C1:** integrar em `main` o trabalho em andamento (`secrets.rs`, `lib.rs`, `commit_review`, `identity_diagnostic`, `work_radar`, `pr_inbox` e o que mais for preciso para compilar) e deixar `cargo check`/`cargo test` verdes.
2. Usuário revisa os ajustes aplicados em `docs/plans/ai-llm.md` (marcados com os IDs desta revisão).
3. Plano de implementação da Fase 0 (ordem sugerida pelo risco: feature `ai` + `ai.toml` → Keychain → catálogo → adaptador OpenAI + teste de fio → erros → Configurações).
