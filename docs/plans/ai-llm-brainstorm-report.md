# Relatório de brainstorming — IA com chave do usuário (BYOK)

| Campo | Valor |
|-------|-------|
| Documento revisado | `docs/plans/ai-llm.md` (versão original de 188 linhas) |
| Processo | Skill `brainstorming` (contexto → NFR → understanding lock → abordagens → design incremental → decision log) + passagem *Skeptic* (modos de falha) |
| Branch / commit base | `agent/ai-brainstorm` @ `fe7b50d` (igual a `main`) |
| Data | 2026-11-08 |
| Modo de execução | Agente sem usuário presente. As perguntas da skill foram registradas com **respostas padrão propostas**; nada aqui está confirmado pelo usuário. |
| Implementação | **Nenhuma.** Nenhum código, crate, comando Tauri ou UI foi criado. |

---

## 1. Contexto: o que existe vs. o que o plano propõe

### 1.1 Base existente relevante

| Peça | Onde | Observação para a IA |
|------|------|----------------------|
| Keychain | `crates/gitpass-core/src/secrets.rs` | Serviço `com.gitpass.GitPass`; contas `github.oauth.<uuid>`; cache em processo + **arquivo marcador** (`tokens/<id>.present`) para saber “existe chave” sem abrir diálogo do Keychain; mutex `KEYCHAIN_IO` serializa prompts; `Debug` redige tokens. Padrão a copiar. |
| Redação | `crates/gitpass-core/src/redact.rs` | `redact_secrets` cobre tokens GitHub, `Bearer`, credenciais em URL, `password=`, chaves privadas. `redact_home` troca a home por `~`. **Não** cobre chaves OpenAI (`sk-…`), Anthropic (`sk-ant-…`), Google (`AIza…`), AWS, Slack, Stripe. |
| Detecção de segredos | `outgoing.rs` (`secret_kind`, `sensitive_file_kind`) | Detecta AWS `AKIA…`, Slack `xox?-`, Stripe `sk_live_`, `.env`, `id_rsa`, `*.pem`. **Catálogo separado do `redact.rs`**: o que é detectado não é necessariamente redigido. |
| Commits | `commit_review.rs` (`CommitReviewRow`) | `sha`, `subject`, autor (nome + e-mail), data, `ownership`, `identity_mismatch`, `reach`. **Sem** arquivos tocados e **sem** corpo da mensagem. |
| Arquivos por commit | `GitRepo::commit_files` | 2× `git diff-tree` + `cat-file` **por commit**; para 30–100 commits seriam 90–300 processos. |
| Radar | `work_radar.rs` | Só **contagens** (staged/unstaged/untracked), sem nomes de arquivos. |
| PR inbox | `pr_inbox.rs` | Número, **título**, URL, repo, estado. **Sem** corpo, labels ou diff. |
| HTTP | `reqwest 0.12` (`blocking`, `json`, `rustls-tls`) | Já há padrão de sanitizar corpo de erro (`github_api::sanitize_error_body`). `GithubApiError::Reqwest` exibe o `reqwest::Error`, cuja mensagem **inclui a URL**. |
| Comandos Tauri | `cmd_*.rs` | `async fn` + `blocking(...)`/`spawn_blocking`; erros como `String`; contrato TS↔Rust em `contract_tests.rs`. |
| CSP | `tauri.conf.json` | `connect-src: ipc: http://ipc.localhost` — a WebView já não consegue chamar hosts externos. |
| Frontend | TS puro, sem lib de markdown | 131 usos de `innerHTML` no código; renderizar texto de LLM por esse caminho seria XSS. |
| Config | `config.toml` (`AppConfig`) | Lido/gravado por app, CLI e daemon; serializado por inteiro (campos desconhecidos são descartados ao regravar). |
| Export de logs | `logs_export.rs` | ZIP com `logs/*.log|*.jsonl` + `recent.json`, tudo redigido. |
| GitPass cria commits? | — | **Não** pela UI (só `commit-tree` interno em branches). O usuário commita no terminal/IDE. |

### 1.2 Lacunas factuais do plano original

1. **Dependência não commitada:** `commit_review.rs`, `work_radar.rs`, `pr_inbox.rs` e `identity_diagnostic.rs` existem apenas como trabalho em andamento no checkout principal; não estão em `main` (`fe7b50d`). A Fase 1 depende deles.
2. “Arquivos tocados (nomes)” não vem do `commit_review`; precisa de coleta nova (idealmente **uma** chamada `git log --name-status -z`).
3. “Resumir PR (título, corpo, labels)” — o app só tem título hoje.
4. “Sugerir mensagem de commit com nomes de arquivos staged” — o radar não tem nomes; e só com nomes a sugestão é genérica.
5. Prompts em `docs/prompts/` não são empacotados no app; precisam ser embutidos no binário.
6. `ai_complete` genérico expõe a chave do usuário a qualquer código que rode na WebView.
7. Gemini com `?key=` na URL + `reqwest::Error` que imprime a URL = chave em mensagens de erro.
8. “Testar conexão” com prompt “Olá” gasta tokens; os três provedores têm endpoint de listagem de modelos (gratuito) que valida a chave.

---

## 2. Requisitos não funcionais (NFR)

Itens marcados **[S]** são suposições propostas, pendentes de confirmação.

| Área | Requisito |
|------|-----------|
| **Fronteira de processo** | Código de IA roda **apenas no processo do app Tauri**. `gitpassd`, CLI, hooks e credential helper **não** linkam nem chamam IA. O NFR de <~50 MB do daemon (DL-012) fica intacto. |
| **Performance** | Montagem de contexto p95 < 1,5 s para 100 commits **[S]**. Chamada: conexão 10 s, total 120 s **[S]** (relatórios em modelos lentos passam de 60 s). UI nunca bloqueia; botão “Cancelar” descarta o resultado. Com IA desligada, custo zero (nada é carregado). |
| **Escala** | 1 usuário, ~5 contas, ~100 repos (DL-013). Uso esperado < 50 chamadas/dia **[S]**. Contexto de metadados ≤ 48 000 caracteres (~12k tokens); com diff ≤ 120 000 caracteres **[S]**. Saída ≤ 1 500 tokens (relatório) / 200 (mensagem) **[S]**. |
| **Privacidade** | Sem servidor GitPass. Envio só ao provedor escolhido, só por clique do usuário, com payload visível antes. Minimização: e-mails nunca enviados; autores pseudonimizados; owner/nome do repo omitidos por padrão **[S]**; caminhos relativos ao repo; diff só opt-in. |
| **Segurança** | Chave só no Keychain; lida só pelo app; nunca devolvida à WebView; nunca em log, erro, `Debug`, ZIP. Saída do LLM tratada como **dado não confiável** (sem HTML, sem ações automáticas). WebView não tem comando de prompt livre. |
| **Confiabilidade** | Falha de IA nunca afeta validação, push, hooks ou qualquer função existente. No máximo 1 retry. Offline → erro claro. |
| **Custo** | Usuário paga. Limite local padrão 20 chamadas/hora **[S]**; teto de tokens de saída; contador local de uso (chamadas + tokens informados pelo provedor). Sem conversão para moeda (preços mudam). |
| **Telemetria** | Continua zero. Envio ao LLM é *egress* iniciado pelo usuário para destino escolhido por ele, com conteúdo exibido — documentado como distinto de telemetria. |
| **Manutenção** | Dono: mantenedor do `gitpass-core`. Versões de API fixadas (ex.: `anthropic-version`), lista curada de modelos revisada a cada release, fixtures por provedor. Sem dependências novas obrigatórias (servidor mock com `tiny_http`, já no projeto). |
| **Idioma** | Respostas em pt-BR por padrão; mensagens de erro em pt-BR. |

---

## 3. Understanding lock

> **Status: NÃO CONFIRMADO.** A skill exige confirmação explícita do usuário antes do design. Nesta rodada sem usuário, o design abaixo foi desenvolvido sobre as respostas padrão e deve ser tratado como **proposta**.

### 3.1 Resumo

- **O quê:** camada opcional de IA *bring your own key* no app desktop, cujo primeiro valor entregue é um **relatório de commits** de um repositório vinculado.
- **Por quê:** transformar dados que o GitPass já coleta (commits, identidade, checks de push) em texto útil (resumo, temas, rascunho de release notes) sem sair do app.
- **Para quem:** desenvolvedor individual com várias contas GitHub (cenário v1) que já tem chave de API de um provedor.
- **Restrições:** telemetria zero; segredos só no Keychain; chave nunca exposta à WebView após digitada; redação obrigatória; núcleo em Rust; macOS 14+; pt-BR.
- **Não-objetivos:** hospedar modelo ou chave; IA no daemon/hooks/CLI; ações automáticas a partir da saída (commit, push, reescrita, abrir links); chat livre; “revisão de segurança por IA”; bloqueio corporativo na v1.
- **Sucesso:** relatório útil em < 60 s, payload previsível e exibido, zero regressão nos NFR atuais, zero vazamento nos testes de corpus.

### 3.2 Suposições

1. O usuário aceita que nomes de arquivos e assuntos de commit saiam da máquina quando ele clica em “Gerar”.
2. As políticas dos provedores (a verificar na data da implementação, pois mudam): OpenAI e Anthropic não treinam com dados da API por padrão; **Gemini no nível gratuito pode usar dados para melhoria do produto e revisão humana** — o nível pago não.
3. A sugestão de mensagem de commit só faz sentido com diff staged; o app não cria commits, então a saída seria apenas “copiar”.
4. Não há demanda imediata de bloqueio corporativo (MDM).
5. O trabalho em andamento do checkout principal (módulos `commit_review`, etc.) será integrado a `main` antes da Fase 1.

### 3.3 Perguntas abertas (com resposta padrão proposta)

| # | Pergunta | Padrão proposto |
|---|----------|-----------------|
| Q1 | Três provedores já na Fase 0 ou um só? | Três adaptadores **mínimos** (texto, sem streaming). |
| Q2 | A sugestão de mensagem de commit entra no roadmap, sabendo que o app não commita e que IDEs já oferecem isso? | Condicional; só depois do diff opt-in; saída só para copiar. |
| Q3 | Diff no relatório? | Não no MVP; opt-in por chamada na Fase 1.1. |
| Q4 | Pseudonimizar autores e omitir owner/nome do repo por padrão? | Sim (ajustável em Configurações). |
| Q5 | Chave Gemini do nível gratuito: bloquear, avisar ou ignorar? | Aviso explícito no consentimento e na tela do provedor. |
| Q6 | Limites padrão? | 20 chamadas/hora; 1 500 tokens de saída; 48k caracteres de contexto. |
| Q7 | Antecipar provedor local (Ollama/OpenAI-compatível em `localhost`)? | Fase 3. |
| Q8 | Bloqueio corporativo via perfil MDM? | Fora da v1; documentar que `enabled = false` não é trava. |
| Q9 | Idioma da resposta fixo pt-BR ou seguir o idioma dos commits? | pt-BR, configurável. |

---

## 4. Abordagens exploradas

### 4.1 Onde roda a IA

| Opção | Prós | Contras |
|-------|------|---------|
| **A1 — Rust no processo do app (`gitpass-core::ai`, chamado só pelo Tauri)** ✅ | Chave nunca na WebView; reaproveita redação/Keychain; CSP intacta; daemon intocado. | `reqwest` blocking dificulta cancelamento real. |
| A2 — `fetch` direto do frontend | Streaming fácil. | Chave na WebView; CSP teria de liberar hosts; viola princípio do plano. |
| A3 — Sidecar/daemon dedicado de IA | Isolamento de processo. | Mais um binário, IPC, assinatura, RAM; YAGNI. |

### 4.2 Provedores

| Opção | Prós | Contras |
|-------|------|---------|
| **B1 — Trait + 3 adaptadores mínimos** ✅ | Cumpre a promessa “sua chave, seu provedor”; cada adaptador é pequeno (montar request, ler texto, ler `usage`, mapear erros). | 3× fixtures e 3× deriva de API. |
| B2 — Um provedor primeiro | Menor esforço inicial. | Produto nasce “só OpenAI”; usuário com outra chave fica de fora. |
| B3 — Um adaptador “OpenAI-compatível” com URLs-base para todos | Um só caminho de código; abre Ollama. | Camadas de compatibilidade de Anthropic/Gemini são de segunda classe (recursos e erros divergentes). Bom para **local** na Fase 3. |

Plano B: se o prazo da Fase 0 apertar, entregar B1 com um adaptador e os outros dois na Fase 1, sem mudar a trait.

### 4.3 Escopo de dados do MVP

| Opção | Prós | Contras |
|-------|------|---------|
| **C1 — Metadados + caminhos, pseudonimizados** ✅ | Baixo risco; payload pequeno e legível na pré-visualização. | Relatório menos profundo (sem ver código). |
| C2 — Diff desde o início | Relatório mais rico. | Código proprietário sai da máquina; redação é *best-effort*; payload grande. |
| C3 — Só assuntos de commit | Risco mínimo. | Valor baixo demais para justificar a feature. |

### 4.4 Decisões menores

- **Configuração:** `ai.toml` separado ✅ vs. seção `[ai]` no `config.toml`. Separado porque daemon/CLI nunca precisam ler preferências de IA, uma versão antiga que regrave `config.toml` não apaga preferências de IA, e “desligar tudo” é apagar um arquivo.
- **Formato da saída:** markdown com seções fixas, exibido como texto ✅ vs. JSON estruturado (suporte desigual nos três provedores, falhas de parse) vs. texto livre (imprevisível).
- **Histórico:** nenhum persistido no MVP ✅ vs. cache em disco (guarda derivados de código proprietário; mais uma coisa para limpar e excluir do ZIP).

---

## 5. Design incremental

### 5.1 Arquitetura e fronteiras

```mermaid
flowchart LR
  UI[WebView TS] -->|invoke por feature| CMD[cmd_ai.rs]
  CMD --> PREP[ai::prepare]
  PREP --> SRC[commit_review / git log]
  PREP --> MIN[pseudonimizar + redigir + limitar]
  MIN --> STORE[(pedidos preparados em memória, TTL 10 min)]
  CMD --> SEND[ai::send]
  SEND --> STORE
  SEND --> KC[Keychain ai.api.*]
  SEND --> PROV[AiProvider: OpenAI / Anthropic / Gemini]
  PROV --> HTTP[reqwest, hosts fixos]
  SEND --> AUD[auditoria local: só metadados]
```

- `gitpass-core/src/ai/` contém: `secrets.rs` (Keychain), `config.rs` (`ai.toml`), `context/` (montadores por feature), `provider/` (trait + 3 adaptadores), `errors.rs` (`GP-AI-*`), `usage.rs`.
- **Não existe comando de prompt livre.** A WebView só pode pedir “preparar relatório do vínculo X com opções Y” e “enviar o pedido preparado Z”. Isso limita o que um XSS ou uma extensão maliciosa conseguiria fazer com a chave do usuário.
- **Preparar → mostrar → enviar:** `prepare` monta o payload final (já redigido), guarda em memória e devolve texto + tamanho + estimativa de tokens + hash. `send(id)` envia **exatamente** aquele payload. O que o usuário vê é o que sai.
- Hosts permitidos são constantes no código (`api.openai.com`, `api.anthropic.com`, `generativelanguage.googleapis.com`). URL-base só é injetável em testes.

### 5.2 Segredos e configuração

- Contas no Keychain: `ai.api.openai`, `ai.api.anthropic`, `ai.api.google` (mesmo serviço `com.gitpass.GitPass`), com **arquivo marcador** e cache como em `secrets.rs`, para `ai_status` não disparar diálogos.
- Tipo `ApiKey(String)` com `Debug`/`Display` redigidos; nunca serializável.
- `ai_set_key` recebe a chave uma vez (campo senha), salva e limpa o campo; nenhum comando devolve a chave, nem os últimos dígitos.
- Validação de formato é **aviso**, não bloqueio (prefixos mudam).
- “Testar conexão” chama o endpoint de listagem de modelos de cada provedor: valida a chave sem gastar tokens.
- Remover chave apaga o item do Keychain e o marcador; revogar no provedor fica com o usuário (link para o console do provedor). Desinstalar o app não apaga itens do Keychain — documentar.
- `ai.toml` (sem segredos): `enabled`, `provider` (vazio até a primeira chave), `model`, `response_locale`, `consent_version`, `diff_consent_version`, `[limits]`, `[privacy]`.

### 5.3 Contexto, minimização e redação

Ordem fixa no `prepare` do relatório:

1. **Escopo:** “commits não enviados” (`@{upstream}..HEAD`) ou “últimos N” (padrão 30, máx. 100).
2. **Coleta:** `commit_review` + **uma** chamada `git log --name-status -z` para os caminhos (no máx. 20 por commit, depois “+N arquivos”).
3. **Minimização:** e-mails removidos; autor vira “você” (`ownership = self_authored`) ou “Autor 1..n”; `identity_mismatch` vai como booleano; owner/nome do repo só se `include_repo_name = true`; caminhos relativos.
4. **Redação:** `redact_for_export` com o **catálogo unificado** de padrões (ver abaixo) em cada campo de texto.
5. **Limite:** se exceder `max_context_chars`, cortar os commits mais antigos e avisar (“12 commits omitidos”).
6. **Serialização determinística** dentro de marcadores (`<dados_do_repositorio>…</dados_do_repositorio>`).

**Catálogo unificado de segredos:** um único conjunto de padrões em `gitpass-core` alimenta detecção (`outgoing`) e redação (`redact`). Hoje são duas listas divergentes. Acrescentar: chaves OpenAI, Anthropic, Google, além dos AWS/Slack/Stripe que já são detectados mas não redigidos.

**Diff (Fase 1.1):** opt-in por chamada; consentimento próprio; exclui arquivos com `sensitive_file_kind`, binários, lockfiles e gerados; teto por arquivo e total; redação linha a linha; se o detector ainda acusar segredo após redação, o arquivo inteiro é retirado e listado na pré-visualização.

### 5.4 Provedores e HTTP

```text
trait AiProvider {
  fn id() -> ProviderId
  fn validate_key(&ApiKey) -> Result<(), AiError>          // GET lista de modelos
  fn complete(&ApiKey, &AiRequest) -> Result<AiResponse, AiError>
}
AiRequest  { system, user, model, max_output_tokens, temperature }
AiResponse { text, finish: Completed|Truncated|Blocked, usage: {input, output}? }
```

| Provedor | Endpoint | Auth |
|----------|----------|------|
| OpenAI | Chat Completions (ou Responses) | `Authorization: Bearer` |
| Anthropic | Messages API, `anthropic-version` fixado | `x-api-key` |
| Gemini | `models/{model}:generateContent` | **header `x-goog-api-key`** (nunca `?key=` na URL) |

- Timeouts: conexão 10 s, total 120 s.
- Retry: no máximo 1, só em 429 (respeitando `Retry-After` ≤ 10 s; acima disso, falha com mensagem) e 502/503/504. Nunca após timeout (o provedor pode já ter cobrado) nem em 400/401/403/404.
- Erros de transporte usam `reqwest::Error::without_url()`; corpo de erro do provedor reduzido ao campo `message` (≤ 200 caracteres) e redigido, como em `sanitize_error_body`.
- Modelos: lista curada embutida (um “recomendado” por provedor, tier rápido/barato) na Fase 0; ID livre na Fase 3. 404 de modelo → `GP-AI-010` sugerindo outro.
- `blocking` mantido no MVP (consistente com o resto do core); “Cancelar” descarta o resultado. Streaming na Fase 3 pode migrar para cliente assíncrono.

### 5.5 Erros (`GP-AI-*`)

| Código | Situação | Ação sugerida ao usuário |
|--------|----------|--------------------------|
| GP-AI-001 | IA desativada | Ativar em Configurações |
| GP-AI-002 | Sem chave para o provedor ativo | Adicionar chave |
| GP-AI-003 | Chave inválida ou revogada (401/403) | Trocar chave |
| GP-AI-004 | Cota/limite do provedor (429, sem crédito) | Ver billing do provedor |
| GP-AI-005 | Limite local atingido | Aguardar ou ajustar limite |
| GP-AI-006 | Rede indisponível / tempo esgotado | Tentar de novo |
| GP-AI-007 | Erro do provedor (5xx) | Tentar mais tarde |
| GP-AI-008 | Contexto grande demais | Reduzir escopo |
| GP-AI-009 | Resposta bloqueada ou vazia (filtro de segurança, recusa) | Ajustar escopo |
| GP-AI-010 | Modelo inexistente/descontinuado | Escolher outro modelo |
| GP-AI-011 | Resposta em formato inesperado | Reportar |
| GP-AI-012 | Keychain negado/indisponível | Permitir acesso |
| GP-AI-013 | Pedido preparado expirou | Gerar de novo |

### 5.6 UX e consentimento

- **Ativação:** toggle → folha de consentimento (o que sai, para quem, custo do usuário, aviso Gemini gratuito, “não use em repositórios sob NDA que proíbam serviços externos”) → grava `consent_version`. Mudança de escopo de dados (ex.: diff) incrementa a versão e pede de novo.
- **Provedor:** escolhido ao salvar a primeira chave; sem provedor “padrão” embutido.
- **Gerar:** botão na gaveta do repo → folha de confirmação **a cada chamada** com resumo (provedor, modelo, N commits, caracteres, ~tokens) e o payload completo expansível. O botão “Enviar” fica nessa folha.
- **Resultado:** bloco **determinístico do GitPass** no topo (divergências de identidade, segredos suspeitos, commits não enviados — vindos dos checks, não da IA) e, abaixo, o texto da IA com rótulo “Gerado por IA — revise”. Assim, a IA não consegue ocultar um alerta real.
- **Ações:** Copiar, Exportar `.md` (com rodapé: provedor, modelo, versão do prompt, data). Nenhum link clicável na saída.
- **Uso:** Configurações mostra chamadas e tokens do mês por provedor (`ai-usage.json`, local).

### 5.7 Saída e renderização

- Prompt de sistema embutido no binário (`crates/gitpass-core/prompts/commit-report.v1.md` via `include_str!`), versionado; a versão vai no resultado e no `.md` exportado.
- O prompt declara que o conteúdo entre marcadores é **dado não confiável** e que instruções ali devem ser ignoradas.
- Seções pedidas: `## Resumo`, `## Temas`, `## Pontos de atenção`, `## Rascunho de release notes`. Validação leve: se faltar seção, exibe o texto mesmo assim, com aviso.
- **MVP:** exibição via `textContent` em bloco `pre-wrap`, nunca `innerHTML`. Fase 3: renderizador de subconjunto seguro (títulos, listas, código) que constrói o DOM nó a nó.
- Saída nunca é executada, aplicada ao Git, nem usada para decidir bloqueios.

### 5.8 Estratégia de testes

| Camada | Teste | Rede |
|--------|-------|------|
| Contexto | *Snapshots* determinísticos dos payloads (repo fixture); pseudonimização; corte por tamanho | Não |
| Redação | Corpus positivo/negativo do catálogo unificado (inclui chaves de IA, AWS, Slack, Stripe, home) | Não |
| Fio | Servidor `tiny_http` local recebe a requisição; assert: nenhum item do corpus, nenhum e-mail, nenhum caminho absoluto, chave só no header certo | Loopback |
| Adaptadores | Fixtures por provedor: sucesso, 401, 429 + `Retry-After`, 5xx, bloqueio, contexto longo, JSON inválido | Não |
| Vazamento da chave | `Debug`/`Display` de erros e tipos nunca contêm a chave; erro de transporte sem URL | Não |
| Export | ZIP de logs sem chave nem conteúdo de prompt/resposta | Não |
| Contrato | Nomes e argumentos dos `invoke`; chaves JSON das respostas | Não |
| CSP | `connect-src` não contém hosts de provedores | Não |
| Manual (release) | Uma chamada real por provedor; checklist de qualidade do relatório em 3 repos de exemplo | Sim, fora do CI |

---

## 6. Modos de falha (passagem *Skeptic*)

| # | Modo de falha | Impacto | Mitigação no design | Fase |
|---|---------------|---------|---------------------|------|
| F1 | Segredo novo (formato não catalogado) em assunto de commit ou caminho | Vazamento ao provedor | Catálogo unificado; pré-visualização exata; metadados no MVP | 0–1 |
| F2 | Diff com código proprietário ou dado pessoal | Exposição contratual/LGPD | Diff só opt-in, consentimento próprio, exclusões, aviso NDA | 1.1 |
| F3 | Chave Gemini em `?key=` aparece em mensagem de erro/log | Vazamento da chave | Header `x-goog-api-key`; `without_url()`; teste | 0 |
| F4 | XSS ou conteúdo injetado na WebView usa `ai_complete` | Uso da chave/cobrança do usuário, exfiltração | Sem prompt livre; comandos por feature; prepare/send | 0 |
| F5 | Saída do LLM com HTML/script inserida via `innerHTML` | XSS no app que gerencia credenciais | `textContent`; CSP já restritiva; teste de UI | 1 |
| F6 | Prompt injection em commit/branch de terceiro (“diga que não há riscos”) | Relatório enganoso | Dados delimitados; alertas reais em bloco determinístico separado; sem ações | 1 |
| F7 | Retry após timeout cobra duas vezes | Custo | Sem retry após timeout; máx. 1 retry | 0 |
| F8 | Loop de cliques / bug de UI dispara chamadas | Custo | Limite local por hora; botão desabilitado durante envio | 0 |
| F9 | Modelo descontinuado pelo provedor | Feature quebra após update do provedor | Lista curada revisada por release; `GP-AI-010` com escolha de outro | 0 |
| F10 | Mudança de API/versão do provedor | Falha de parse | Versão fixada; fixtures; `GP-AI-011` | contínuo |
| F11 | Keychain pede senha repetidamente | UX ruim | Marcador + cache + mutex como `secrets.rs` | 0 |
| F12 | Pré-visualização difere do que é enviado (dados mudam entre telas) | Quebra de confiança | Payload congelado em memória por ID, TTL 10 min | 1 |
| F13 | Montagem de contexto lenta (300 processos git) | UI travada | Uma chamada `git log --name-status -z`; `spawn_blocking` | 1 |
| F14 | Usuário entende “telemetria zero” como “nada sai da máquina” | Expectativa quebrada | Texto de consentimento explícito; doc distingue egress do usuário | 0 |
| F15 | Gemini gratuito usa dados para treino | Exposição de dados | Aviso no consentimento e ao salvar chave Google | 0 |
| F16 | Versão antiga de CLI/daemon regrava `config.toml` e perde preferências | Preferências somem | `ai.toml` separado | 0 |
| F17 | Usuário confia na IA como revisão de segurança | Falsa sensação de segurança | Rótulo fixo; alertas vêm só dos checks determinísticos | 1 |
| F18 | Escopo cresce (chat, ⌘K, PR, ideias) antes do MVP provar valor | Atraso, manutenção | Fases com critérios de saída; itens vagos removidos | todas |

---

## 7. Decision log

Status: **Proposta** = padrão aplicado sem confirmação do usuário.

| ID | Decisão | Alternativas consideradas | Motivo | Status |
|----|---------|---------------------------|--------|--------|
| AI-DL-001 | IA roda só no processo do app (Rust); nunca em daemon, CLI, hooks, helper | Frontend direto; sidecar dedicado | Chave fora da WebView; RAM do daemon; hooks sem latência de rede | Proposta |
| AI-DL-002 | Trait `AiProvider` + 3 adaptadores mínimos na Fase 0 | Um provedor; adaptador único OpenAI-compatível | Promessa BYOK; adaptadores pequenos; compatibilidade de 2ª classe | Proposta (Q1) |
| AI-DL-003 | Sem comando de prompt livre; comandos por feature com prepare/send | `ai_complete(prompt)` genérico | Limita abuso da chave por código na WebView; payload exibido = enviado | Proposta |
| AI-DL-004 | Preferências em `ai.toml` separado | Seção `[ai]` em `config.toml` | Isolamento de daemon/CLI; sem perda ao regravar; desligar = apagar arquivo | Proposta |
| AI-DL-005 | Chaves no Keychain `ai.api.<provedor>` com marcador + cache | Item único JSON com as três chaves | Remoção independente; padrão já testado em `secrets.rs` | Proposta |
| AI-DL-006 | Testar conexão via listagem de modelos | Prompt “Olá” | Sem custo; valida a chave do mesmo jeito | Proposta |
| AI-DL-007 | Gemini autentica por header | `?key=` na URL | URL aparece em erros/logs | Proposta |
| AI-DL-008 | MVP envia só metadados + caminhos | Diff desde o início; só assuntos | Risco baixo com valor suficiente | Proposta (Q3) |
| AI-DL-009 | Pseudonimizar autores, nunca enviar e-mails, omitir owner/repo por padrão | Enviar tudo | Minimização de dados pessoais de terceiros e de nomes de clientes | Proposta (Q4) |
| AI-DL-010 | Catálogo único de padrões de segredo para detecção e redação, com chaves de IA | Manter duas listas | Hoje há segredos detectados que não são redigidos | Proposta |
| AI-DL-011 | Confirmação com payload a cada chamada | Só na primeira por sessão | Cada chamada tem custo e pode ter escopo diferente | Proposta |
| AI-DL-012 | Alertas reais em bloco determinístico separado do texto da IA | Pedir à IA para listar riscos | IA pode omitir ou ser manipulada por injeção | Proposta |
| AI-DL-013 | Saída exibida como texto (`textContent`) no MVP | Renderizar markdown com `innerHTML` | Evita XSS; renderizador seguro fica para a Fase 3 | Proposta |
| AI-DL-014 | Markdown com seções fixas | JSON estruturado; texto livre | Funciona igual nos 3 provedores; falha graciosa | Proposta |
| AI-DL-015 | Prompts embutidos no binário e versionados | Arquivos em `docs/prompts/` | `docs/` não é empacotado; versão rastreável no resultado | Proposta |
| AI-DL-016 | Timeouts 10 s / 120 s; máx. 1 retry; nunca após timeout | 60 s + backoff genérico | Modelos lentos; evitar cobrança dupla | Proposta |
| AI-DL-017 | Limite local 20/hora + teto de saída + contador de tokens | 10/hora; estimativa em moeda | Protege custo sem atrapalhar; preços mudam | Proposta (Q6) |
| AI-DL-018 | Sem histórico persistido no MVP | Cache das últimas 20 respostas | Derivados de código proprietário em disco; YAGNI | Proposta |
| AI-DL-019 | Sem provedor padrão embutido | `openai` como padrão | Neutralidade; o provedor é o da primeira chave | Proposta |
| AI-DL-020 | Remover “Explicar bloqueio GP-*” do escopo de IA | Manter como assist | Texto determinístico é mais preciso, gratuito e offline | Proposta |
| AI-DL-021 | Remover “Ideias de próximo passo” | Manter | Sem critério de sucesso; contexto vago | Proposta |
| AI-DL-022 | “Resumir PR” adiado para a Fase 3, condicionado a diff via API | Fase 2 só com título | Só título não gera valor | Proposta |
| AI-DL-023 | Sugestão de mensagem de commit condicional, depois da Fase 1.1 | Fase 2 só com nomes | Sem diff a mensagem é genérica; app não commita; IDEs já fazem | Proposta (Q2) |
| AI-DL-024 | Modo de emergência não interage com IA | Bloquear ou avisar | IA não toca identidade nem hooks | Proposta |
| AI-DL-025 | Bloqueio corporativo fora da v1; documentar que config não é trava | `ai.enabled` “travado” | Arquivo editável pelo usuário não é política; MDM é outro projeto | Proposta (Q8) |
| AI-DL-026 | Testes de fio com `tiny_http` local, sem `httpmock` | Nova dependência de dev | Já existe no projeto | Proposta |
| AI-DL-027 | Integrar o trabalho em andamento (`commit_review` etc.) em `main` é pré-requisito da Fase 1 | Reimplementar no branch de IA | Evita duplicação e conflito | Proposta (Q-processo) |

---

## 8. Principais riscos

1. **Vazamento de segredo ou código ao provedor** — redação é *best-effort*. Mitigado por metadados no MVP, catálogo unificado, exclusões no diff, payload exato exibido, testes de corpus (AI-DL-008/010/011).
2. **Vazamento da própria chave de IA** — erros de `reqwest` com URL, `?key=` do Gemini, `Debug`. Mitigado por header, `without_url()`, tipo `ApiKey` redigido, testes (AI-DL-005/007).
3. **Abuso da chave ou XSS a partir da WebView** — `ai_complete` genérico e saída em `innerHTML`. Mitigado por comandos por feature, prepare/send e `textContent` (AI-DL-003/013).
4. **Relatório enganoso por prompt injection ou alucinação** — mitigado por dados delimitados, bloco determinístico separado e rótulo “revise” (AI-DL-012).
5. **Custo e deriva de provedores** — modelos descontinuados, mudanças de API e de política de dados, cobrança inesperada. Mitigado por limites locais, uso visível, versões fixadas, lista curada e fixtures (AI-DL-016/017).

---

## 9. Critérios de saída da skill

| Critério | Status |
|----------|--------|
| Understanding lock confirmado | **Não** — aguardando o usuário (seção 3) |
| Pelo menos uma abordagem aceita | **Não** — recomendadas A1/B1/C1, sem aceite explícito |
| Suposições documentadas | Sim (seções 2 e 3.2) |
| Riscos reconhecidos | Sim (seções 6 e 8) |
| Decision log completo | Sim, como proposta (seção 7) |

Pela skill, **a implementação não está autorizada** até o usuário confirmar a seção 3 e aceitar as abordagens. Como o design envolve privacidade e credenciais (alto impacto), a skill também pede uma revisão `multi-agent-brainstorming` do design final antes da Fase 0.

## 10. Próximos passos (fora desta rodada)

1. Usuário responde Q1–Q9 (ou aceita os padrões) e confirma o understanding lock.
2. Integrar em `main` os módulos em andamento do checkout principal.
3. Revisão `multi-agent-brainstorming` do design confirmado.
4. Só então: plano de implementação da Fase 0.
