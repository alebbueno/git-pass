# GitPass — Design v1 (macOS)

## Understanding summary

- **Produto:** App desktop **macOS 14+** para gerenciar **múltiplas contas GitHub**, com **agente em background** (login item) e **menu bar**. Cada **repositório local** fica associado a **uma conta**; objetivo é impedir **commits/push** com **identidade ou remoto incompatível** e dar **referência visual** da conta do projeto atual.
- **Stack:** **Rust** no núcleo (**daemon** `gitpassd`) + **Tauri** para UI (WebView). UI “**Liquid Glass**” como **aproximação** visual (blur/tema), sem promessa de paridade **AppKit/SwiftUI**.
- **Interceptação Git:** **Git hooks** via **`core.hooksPath` global** (caminho gerenciado) + **`gitpass` CLI**; **wrapper/`git` shim opcional** no PATH para casos sem hooks. Validação **centralizada no daemon** (IPC via **socket Unix**).
- **Autenticação por conta:** usuário escolhe **SSH ou HTTPS** por conta; **OAuth no navegador** + armazenamento no **Keychain**; chaves SSH em `~/.ssh` com permissões corretas.
- **Validação `origin` vs conta:** regras **locais obrigatórias**; para **ambiguidades** (forks, transferências, etc.), **API GitHub** quando online; se API necessária e indisponível → **permite com aviso forte** + estado **“não confirmado”** (log local).
- **Pontos de bloqueio:** operações que **criam commit** (`commit`, `merge`, `rebase --continue`, `cherry-pick`, `revert`, …) e **`push`**, conforme política acordada quando agente saudável.
- **Resiliência:** se **daemon/offline IPC** → **`push` bloqueado**; **`commit` local** só com **modo emergência** (**env** e/ou **toggle na menu bar com timer** + **auditoria**). **CI:** bypass só com **detecção de CI** **e** `GITPASS_ALLOW_CI=1`.
- **NFR:** ~**<50 MB RAM** idle para o agente; escala v1 **~5 contas** / **~100 repos**; **telemetria zero**; logs **locais exportáveis**.

## Assumptions

- OAuth GitHub com **PKCE** e escopos **mínimos** (lista a fechar na implementação).
- **Um usuário humano** por máquina como cenário principal.
- Instalação aceita ajuste de **config Git global** (`core.hooksPath`) e opcionalmente **PATH** para wrapper.
- **Menu bar “contexto atual”** pode usar **Accessibility** opcionalmente para inferir repo (VS Code/Terminal); se negado, cai-se para **último repo** + **pin**.

## Open questions (pós-v1 ou definição fina antes do build)

- **Idioma da UI:** PT-BR apenas vs. i18n mínimo.
- **Distribuição:** build local vs. **assinatura + notarização** + canal de updates.
- **Detalhe de UX** para **submódulos** e **múltiplos remotes** além `origin` (v1 pode documentar limites).

## Architecture

### Processos

| Componente | Função |
|------------|--------|
| `gitpassd` | Login Item / daemon Rust: IPC, Keychain, regras, API GitHub, SSH/credential wiring. |
| App Tauri | UI: onboarding, contas, repos, integração, diagnóstico; menu bar / tray. |
| `gitpass` CLI | Chamado por hooks e usuário; fala com `gitpassd` (socket), timeouts + retry. |
| Hooks | Scripts finos em dir. global (via `core.hooksPath`) → `gitpass hook …`. |
| Wrapper (opt-in) | `git` delegável para binário real após `validate` nas fases críticas. |

### Persistência

- **Keychain:** tokens OAuth, PAT, passphrases de chave.
- **Arquivo local (não sensível):** bindings `repo_root → account_id`, pin, preferências, estado de emergência com expiração, hosts SSH aliases.
- **~/.ssh:** material de chave; app referencia e valida permissões.

### Menu bar — “repo atual” (prioridade)

1. **Repo pinado** (vence quando ativo).  
2. **Foco** (Accessibility): app em primeiro plano → inferir cwd/repo quando permitido.  
3. **Último repo** validado por hook/CLI em janela recente.

### Validação (pipeline)

1. Resolver **root**, **binding de conta**, **`origin`**, **modo auth**.  
2. **Checagens locais** (host, owner, consistência SSH/HTTPS).  
3. Se **ambíguo** → **API** com token da conta.  
4. Falha de API quando necessária → **allow + aviso** + **não confirmado** (conforme política).  
5. Erros com **código** (`GP-*`), causa, ação sugerida.

### Instalação / integração

- **`git config --global core.hooksPath`** apontando para diretório gerenciado (com backup/`doctor` para conflitos).
- **`gitpass doctor`:** daemon, hooks, ssh, helper, bindings, macOS version, Accessibility.
- **Wrapper:** opt-in documentado; nunca sobrescreve `/usr/bin/git` do sistema.

### UI / “Liquid Glass”

- Backdrop **blur** + tema claro/escuro alinhado ao macOS; tipografia e espaçamento **HIG-like** na medida do Web stack.
- Notificações **sem segredos** (apenas códigos, nomes de repo).

### Logs

- Rotação local; export **ZIP** com manifesto; **redigir** tokens/chaves.

### Segurança

- Socket IPC: permissões restritas, path sob **Application Support**, validação de cliente básica.

## Risks and mitigations

| Risco | Mitigação |
|-------|-----------|
| Ferramentas ignoram hooks | Wrapper opt-in + documentação. |
| Forks / renomeações | API + overrides explícitos na UI (fase posterior se necessário). |
| Accessibility negada | Fallback último/pin. |
| RAM / idle | Daemon separado do WebView; lazy UI. |
| IPC comprometido localmente | Permissões mínimas, superfície pequena; documentar modelo de ameaça. |

## Decision log

| ID | Decision |
|----|----------|
| DL-001 | Escopo v1: **GitHub apenas**. |
| DL-002 | `origin`/conta incompatível → **bloqueio** em commit-criação e push (quando agente saudável e política aplicável). |
| DL-003 | Integração: **hooks** + **wrapper opt-in**. |
| DL-004 | Menu bar contexto: **pin > foco (Accessibility) > último repo**. |
| DL-005 | Identidade: **conta por repositório** (binding). |
| DL-006 | Auth por conta: **SSH ou HTTPS** (escolha por conta). |
| DL-007 | Validação: **local obrigatório** + **API** para ambíguos. |
| DL-008 | Background: **login item**, agente **sempre ativo**. |
| DL-009 | Bloqueio em **criação de commit** + **`push`**. |
| DL-010 | UI: **Tauri**; glass = **aproximação**. |
| DL-011 | Segredos **Keychain**; config local não sensível; SSH em `~/.ssh`. |
| DL-012 | Performance: **<~50 MB** daemon idle. |
| DL-013 | Escala v1: **~5 contas**, **~100 repos**. |
| DL-014 | API indisponível em caso ambíguo → **allow com aviso** + **não confirmado**. |
| DL-015 | Daemon off: **`push` bloqueado**; **commit** local só emergência. |
| DL-016 | Emergência: **env** + **toggle timer** + **auditoria**. |
| DL-017 | CI bypass: **detecção CI + `GITPASS_ALLOW_CI=1`**. |
| DL-018 | **Zero telemetria** v1. |
| DL-019 | macOS mínimo: **14 Sonoma**. |
| DL-020 | Understanding lock: **confirmado**. |
| DL-021 | Arquitetura: **agente headless + Tauri**. |
| DL-022 | IPC: **socket Unix** + CLI; validação no daemon. |
| DL-023 | Estado não sensível em arquivo; segredos no Keychain. |
| DL-024 | Bloco de design 1 (visão/fronteiras): **aceito**. |
| DL-025 | OAuth = **PKCE + Keychain**; escopos mínimos na implementação. |
| DL-026 | Binding de repo = path normalizado do root `.git`; mudança de path exige reconciliação. |
| DL-027 | Menu bar: prioridade **pin > foco Accessibility > último repo**. |
| DL-028 | Bloco de design 2 (onboarding/menu bar): **aceito**. |
| DL-029 | Bloco de design 3 (validação/API/erros): **proposto** — confirmar na revisão final. |
| DL-030 | Hooks: **`core.hooksPath` global** gerenciado. |
| DL-031 | Wrapper: **opt-in PATH**. |
| DL-032 | IPC fail: timeout + retry; política emergência/push. |
| DL-033 | UI Tauri; glass aproximado; disclaimer não-paridade nativa. |
| DL-034 | Notificações sem segredos. |
| DL-035 | Export logs redigido. |
| DL-036 | IPC hardening básico. |
| DL-037 | Blocos de design 4 (hooks/doctor/wrapper) e 5 (UI/riscos): **propostos** — confirmar na revisão final. |

### Status dos blocos de design

| Bloco | Tópico | Status |
|-------|--------|--------|
| 1 | Visão geral e fronteiras (daemon + Tauri + IPC) | **Confirmado** |
| 2 | Onboarding OAuth, binding repo↔conta, menu bar | **Confirmado** |
| 3 | Pipeline de validação, API, degradação, mensagens `GP-*` | **Aguardando sua confirmação** |
| 4 | `core.hooksPath`, `doctor`, wrapper opt-in, timeout IPC | **Aguardando sua confirmação** |
| 5 | UI Tauri “glass”, notificações, logs, riscos | **Aguardando sua confirmação** |

## Exit criteria (brainstorming)

- [x] Understanding lock confirmado  
- [x] Pelo menos uma abordagem arquitetural escolhida (agente + Tauri)  
- [x] Assumptions documentadas  
- [x] Riscos principais listados  
- [x] Decision log completo  

## Implementation handoff (próximo passo, fora deste documento)

Quando quiser começar o build: plano incremental (daemon IPC → CLI/hooks → OAuth → bindings → Tauri shells → notarização opcional).

Se o escopo evoluir para **alto risco** (segurança corporativa, múltiplos provedores), considerar revisão cruzada adicional antes do primeiro release.
