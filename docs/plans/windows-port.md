# Plano: build e portabilidade Windows

> **Status:** atualizado 2026-11-11. **Fase W0 em andamento** — CI `build-windows.yml`, bundle NSIS, sidecars `.exe` e UX básica (pastas, Explorer, logs). Paridade de produto (daemon, hooks, integração Git) ainda **macOS 14+** como referência (ver [README](../../README.md)).

## Objetivo

1. Permitir **compilar e empacotar** o GitPass Desktop no Windows (instalador ou binário portátil), via **máquina Windows** ou **CI** (`windows-latest`).
2. Definir um roteiro incremental até **paridade razoável** com o macOS: contas OAuth, segredos, integração Git, daemon, tray e diagnóstico.

## Não-objetivos (fase inicial / smoke)

- Cross-compile **macOS → Windows** como fluxo oficial de release (o Tauri espera o host nativo para bundle e WebView2).
- Paridade 100% com macOS na primeira entrega Windows (menu bar estilo macOS, `launchd`, Automator, etc.).
- Assinatura de código e notarização no primeiro smoke (documentar depois; recomendado antes de distribuição pública).

## Situação atual (baseline)

| Área | macOS hoje | Windows hoje |
|------|------------|--------------|
| CI / release | `.github/workflows/build-gate.yml` → `macos-14`, `scripts/verify-app-build.sh` | `.github/workflows/build-windows.yml` + `scripts/verify-app-build.ps1` |
| Tauri bundle | `targets: ["app", "dmg", "nsis"]` em `gitpass-desktop/src-tauri/tauri.conf.json` | NSIS no host Windows (smoke CI) |
| Sidecars | `gitpass-desktop/scripts/build-sidecars.mjs` + `src-tauri/binaries/*-{triple}` | Triple `x86_64-pc-windows-msvc` (e opcionalmente `aarch64-pc-windows-msvc`) não gerado no pipeline |
| Segredos | Keychain (`keyring`, serviço `com.gitpass.GitPass`) | `keyring` pode usar Credential Manager, mas UI/copy ainda falam “Keychain do macOS” |
| Integração Git | `credential.helper` + `osxkeychain`, hooks, `launchd` | Stubs / erros em `#[cfg(not(unix))]` em `integration.rs`, `hooks.rs`, `ipc_client.rs` |
| Daemon `gitpassd` | Socket Unix em Application Support | `ipc_client::request` → `DaemonUnreachable::NotRunning` em `#[cfg(not(unix))]` |
| App Tauri | Tray, dock, notificações (`notify-rust`), `osascript` para pastas | Vários comandos retornam *“Só funciona no macOS”* (`cmd_repos.rs`, `main.rs`, `cmd_cloud.rs`, etc.) |
| Dados do app | `dirs::data_dir()` + `com.gitpass.GitPass` (funciona em Win) | Caminhos ok; documentação ainda cita `~/Library/...` |

**Conclusão:** é **possível** um **smoke build** (shell Tauri + sidecars compilando) em Windows antes da paridade; um **produto utilizável** exige as fases abaixo.

---

## Por que não gerar Windows a partir do Mac

1. **Tauri bundle** no Windows precisa de **WebView2**, toolchain **MSVC** e bundler **NSIS/MSI/WiX** no ambiente Windows.
2. O script de sidecars usa o triple do host ou `TAURI_ENV_TARGET_TRIPLE`; fazer DMG no Mac **não** produz `.exe`/`.msi`.
3. Vários crates usam `#[cfg(unix)]` para integração real; compilar para Windows pode até passar em partes do workspace, mas o **fluxo de produto** (daemon, hooks, credencial) não está ligado.

Cross-compile pontual (`--target x86_64-pc-windows-msvc` no macOS) não é recomendado como release gate deste repositório.

---

## Pré-requisitos (máquina Windows — dev ou CI)

| Ferramenta | Notas |
|------------|--------|
| **Windows 10/11** (x64; ARM64 opcional depois) | Host de build |
| **Rust stable** (`rustup`) | Perfil `x86_64-pc-windows-msvc` |
| **Visual Studio Build Tools** | C++ workload (linker MSVC) |
| **Node.js 20+** | `gitpass-desktop/package-lock.json` |
| **WebView2 Runtime** | Em dev, instalado no sistema; o instalador Tauri pode embutir bootstrapper |
| **Git for Windows** | Para testar integração e hooks localmente |

Opcional para assinatura de release:

- Certificado de assinatura de código (Authenticode) + `signCommand` no Tauri (configurar quando houver distribuição pública).

---

## Instruções: smoke build local (Windows)

Quando o repositório tiver os ajustes mínimos da **Fase W0** (ver abaixo), o fluxo esperado é:

```powershell
# Na raiz do repositório
cd gitpass-desktop
npm ci
npm run build:app
# Sidecars: gitpass-desktop\src-tauri\binaries\gitpass-*-x86_64-pc-windows-msvc.exe
cargo test -p gitpass-desktop contract
npm run tauri build
```

Artefatos típicos do Tauri 2 (após configurar `bundle.targets`):

- `target\release\bundle\nsis\GitPass_*_x64-setup.exe`, e/ou
- `target\release\bundle\msi\GitPass_*_x64_en-US.msi`

**Variáveis úteis**

```powershell
$env:CARGO_TARGET_DIR = "D:\gitpass-target"   # opcional, espelha CARGO_TARGET_DIR no macOS
$env:TAURI_ENV_TARGET_TRIPLE = "x86_64-pc-windows-msvc"  # usado por build-sidecars.mjs se necessário
```

**Verificação rápida sem instalador**

```powershell
cd gitpass-desktop
npx tsc --noEmit
npm run build
cargo build -p gitpass-desktop --release
```

---

## Instruções: CI GitHub Actions (recomendado)

Workflow: `.github/workflows/build-windows.yml` (não substitui o gate macOS até a paridade mínima).

**Disparo manual:** Actions → **build-windows** → **Run workflow** (qualquer branch). Artefato **gitpass-windows-x64**. Opcional: marcar **Publicar… GitHub Release** para anexar o NSIS a um release (tag customizada ou `v<versão>-windows-<run>`).

Esboço histórico do job (referência):

```yaml
# Esboço — .github/workflows/build-windows.yml (futuro)
name: build-windows
on:
  workflow_dispatch:
  pull_request:
    paths: [gitpass-desktop/**, crates/**, Cargo.toml, Cargo.lock]
jobs:
  smoke:
    runs-on: windows-latest
  timeout-minutes: 60
  steps:
    - uses: actions/checkout@v4
    - uses: actions/setup-node@v4
      with:
        node-version: "20"
        cache: npm
        cache-dependency-path: gitpass-desktop/package-lock.json
    - run: npm ci
      working-directory: gitpass-desktop
    - uses: dtolnay/rust-toolchain@stable
      with:
        targets: x86_64-pc-windows-msvc
    - name: Smoke (sem prometer paridade)
      env:
        CARGO_TARGET_DIR: ${{ github.workspace }}/target/ci-windows
      run: |
        cd gitpass-desktop
        npx tsc --noEmit
        npm run build:app
        cargo test -p gitpass-desktop contract
        npm run tauri build
    - uses: actions/upload-artifact@v4
      with:
        name: gitpass-windows-x64
        path: |
          target/ci-windows/release/bundle/nsis/*.exe
          target/ci-windows/release/bundle/msi/*.msi
```

Critério de sucesso **W0:** workflow verde + artefato anexado; app abre; telas que não dependem de integração/daemon funcionam.

---

## Ajustes de configuração (Fase W0 — só empacotar)

Checklist de arquivos:

1. **`gitpass-desktop/src-tauri/tauri.conf.json`**
   - Em macOS, manter `app` + `dmg`.
   - Para Windows: `bundle.targets` incluir `nsis` e/ou `msi` (pode usar perfil condicional por CI ou documentar dois comandos).
   - Garantir `icons/icon.ico` na lista `bundle.icon` (já existe `icon.ico` no repositório).
   - `externalBin`: os sidecars devem existir como `gitpass-cli-x86_64-pc-windows-msvc.exe`, etc. (sufixo `.exe` no Windows).

2. **`gitpass-desktop/scripts/build-sidecars.mjs`**
   - Já suporta `--target` e `TAURI_ENV_TARGET_TRIPLE`; validar nomes `.exe` na cópia para `binaries/`.
   - No Windows, `chmodSync` é inofensivo; testar path `CARGO_TARGET_DIR`.

3. **`scripts/verify-app-build.sh`**
   - Manter macOS-only; opcional futuro: `scripts/verify-app-build.ps1` espelhando passos (tsc, build:app, contract, tauri build).

4. **README / créditos**
   - Mencionar que Windows é preview até Fase W2.

---

## Fases de produto Windows

### Fase W0 — Smoke build (1–2 PRs)

- [x] CI `windows-latest` + artefato NSIS (MSI opcional depois).
- [x] Sidecars Windows: sufixo `.exe` em `build-sidecars.mjs`.
- [ ] App inicia; navegação, configurações, temas (validar no runner / VM).
- [x] Mensagens honestas na UI quando recurso não existir no Windows (daemon, integração Git).

**Gate:** `npx tsc`, `contract` tests, `tauri build` no runner Windows.

### Fase W1 — Segredos e contas (paridade OAuth)

- [ ] Validar `keyring` no Windows (Credential Manager) para `github.oauth.*` e `ai.api.*` (mesmo `KEYCHAIN_SERVICE` / nomes de conta).
- [ ] Copy e diagnóstico: trocar “Keychain do macOS” por texto neutro ou específico de plataforma (`open_keychain_docs`, settings, IA).
- [ ] Testes manuais: conectar conta GitHub, reiniciar app, token persiste.

**Referências:** `crates/gitpass-core/src/secrets.rs`, `crates/gitpass-core/src/ai/secrets.rs`, `gitpass-desktop/src-tauri/src/cmd_accounts.rs`.

### Fase W2 — Integração Git + daemon (núcleo do produto)

| macOS | Direção Windows |
|-------|-----------------|
| Socket Unix `gitpassd.sock` | Named pipe ou TCP localhost (definir contrato em `ipc_client.rs` + `gitpassd`) |
| `launchd` plist | Tarefa agendada no logon, serviço Windows, ou daemon iniciado pelo app — **decisão de produto** |
| `git config --global credential.helper` + helper bundled | Equivalente: `manager` / `manager-core` do Git for Windows **ou** helper GitPass no PATH; evitar conflito com helpers existentes |
| Hooks `core.hooksPath` | Mesma ideia; paths e permissões Win |
| `ping_daemon` / integração | Implementar `ping_daemon` e instalação fora de `#[cfg(unix)]` |

**Referências:** `crates/gitpass-core/src/integration.rs`, `crates/gitpass-core/src/hooks.rs`, `crates/gitpass-core/src/ipc_client.rs`, `crates/gitpassd/src/main.rs`.

**Decisão pendente (registrar quando fechada):** modelo de daemon — usuário vs serviço de sistema vs só IPC in-process no app (impacta hooks e credencial em background).

### Fase W3 — Shell desktop (UX Windows)

- [x] Seletor de pastas: `rfd` em `cmd_repos.rs` (macOS mantém `osascript`).
- [x] Abrir pasta / Explorer / terminal / editor: `explorer` e `cmd start` em `cmd_repos.rs`; pasta de dados em `main.rs`.
- [ ] Tray: validar comportamento Tauri 2 no Windows; notificações sem `notify-rust` mac-only (dependência já em `target.'cfg(macos)'`).
- [ ] Atalhos: `Ctrl` em vez de `⌘` onde o front usa meta (paleta, tour).
- [ ] `focus.rs`, `pending_alert.rs`, `dock.rs` — revisar ou guardar atrás de `cfg(macos)`.

### Fase W4 — Diagnóstico, CLI e release

- [ ] `diagnostics.rs`: checks específicos Win (Git no PATH, helper, pipe do daemon, permissões).
- [ ] `gitpass` / `gitpassd` no PATH ou instalador adiciona ao PATH.
- [ ] Assinatura de código + atualizações (opcional Tauri updater).
- [ ] README: pré-requisitos Windows, limitações conhecidas.

---

## Matriz de funções (futuro próximo)

Legenda: **S** smoke (W0), **P** paridade planejada (W1–W4), **—** fora de escopo v1 Windows.

| Função | Fase | Notas |
|--------|------|--------|
| UI geral / temas / rotas | S | Já cross-platform |
| Contas GitHub OAuth | P | W1 — Credential Manager |
| IA BYOK | P | W1 — mesmas regras que macOS; chave não volta à WebView |
| Repositórios / mapa / detalhe | P | W3 — picker de pasta; `open` no Explorer |
| Integração global Git | P | W2 — helper + `useHttpPath` equivalente |
| Daemon + validação em push/commit | P | W2 — IPC Win |
| Tray + segundo plano | P | W3 |
| Diagnóstico completo | P | W4 |
| Menu bar exclusivo macOS | — | Windows usa tray/janela |
| `launchd` / Automator | — | Substituir por mecanismo Win |

---

## Riscos e mitigações

| Risco | Mitigação |
|-------|-----------|
| Helper Git conflita com GCM (`manager-core`) | Detectar em `credential_state`; documentar ordem de helpers; testes em VM limpa |
| Daemon não sobe no logon | Instalador configura tarefa; app mostra estado “serviço parado” com ação “Iniciar” |
| Keyring pede UI estranha no Windows | Mutex + marcadores (mesmo padrão do macOS); testes E2E manuais |
| Dois targets (x64 vs ARM64) | Começar só `x86_64-pc-windows-msvc`; ARM64 quando houver demanda |
| Divergência CI macOS vs Windows | Manter `build-gate` macOS obrigatório; Windows smoke não bloqueia main até W0 estável |

---

## Critérios de “pronto para beta Windows”

1. CI Windows verde com instalador anexado.
2. Usuário conecta conta GitHub e token persiste após reboot.
3. Integração instala helper + hooks; push/pull em repo de teste passa na conta correta.
4. Diagnóstico lista falhas acionáveis em português.
5. Documentação de suporte atualizada (`settings/credits`, README).

---

## Referências no repositório

| Tópico | Caminho |
|--------|---------|
| Config Tauri / bundle | `gitpass-desktop/src-tauri/tauri.conf.json` |
| Sidecars | `gitpass-desktop/scripts/build-sidecars.mjs`, `crates/gitpass-core/src/integration.rs` (`SIDECARS`) |
| Build release macOS | `scripts/verify-app-build.sh` |
| CI atual | `.github/workflows/build-gate.yml` |
| Stubs não-macOS (amostra) | `gitpass-desktop/src-tauri/src/cmd_repos.rs`, `main.rs`, `crates/gitpass-core/src/ipc_client.rs` |
| Segredos | `crates/gitpass-core/src/secrets.rs` |
| Deps só macOS no app | `gitpass-desktop/src-tauri/Cargo.toml` (`notify-rust`, `core-foundation`) |

---

## Registro de decisões (preencher ao implementar)

| ID | Decisão | Data | Notas |
|----|---------|------|-------|
| WIN-DL-001 | IPC Windows: named pipe vs TCP localhost | — | Afeta `gitpassd` e firewall |
| WIN-DL-002 | Daemon: serviço Windows vs tarefa de logon vs só app | — | Afeta hooks fora do app |
| WIN-DL-003 | Instalador padrão: NSIS vs MSI | 2026-11-11 | Começar com **NSIS** no CI |
| WIN-DL-004 | Convivência com Git Credential Manager | — | Padrão em Git for Windows |

---

## Ordem sugerida de trabalho (próximo sprint)

1. **W0:** `tauri.conf.json` (ícones + targets Win), validar sidecars `.exe`, workflow `build-windows.yml`.
2. **W1:** testes de `keyring` + textos de UI.
3. **Decisões WIN-DL-001 e WIN-DL-002** antes de codar W2.
4. **W2:** IPC + integração (maior esforço).
5. **W3/W4:** polish e diagnóstico.

Quem for executar o primeiro build Windows deve começar pela **Fase W0** neste documento e anexar o log do CI (ou caminho do `.exe`) na PR correspondente.
