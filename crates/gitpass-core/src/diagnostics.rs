//! Diagnostics shown in the app (`system_diagnostics`) and printed by `gitpass doctor`: the Git
//! integration status plus Git, Keychain, SSH, bindings, macOS and Accessibility checks.
//! Types match `Diagnostics` / `DiagnosticItem` in `api-system.ts`.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::Serialize;
use uuid::Uuid;

use crate::audit::now_rfc3339;
use crate::config::AppConfig;
use crate::integration::{self, CheckState, ComponentStatus, IntegrationEnv, IntegrationStatus};
use crate::validate::binding_health;
use crate::AuthMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticState {
    Ok,
    Warning,
    Error,
    Info,
}

impl DiagnosticState {
    pub const ALL: [DiagnosticState; 4] =
        [DiagnosticState::Ok, DiagnosticState::Warning, DiagnosticState::Error, DiagnosticState::Info];
}

pub const ACTION_REPAIR_INTEGRATION: &str = "repair_integration";
pub const ACTION_RESTART_DAEMON: &str = "restart_daemon";
pub const ACTION_OPEN_ACCESSIBILITY: &str = "open_accessibility";
pub const ACTION_OPEN_ACCOUNTS: &str = "open_accounts";
pub const ACTION_OPEN_REPOS: &str = "open_repos";
pub const ACTIONS: [&str; 5] = [
    ACTION_REPAIR_INTEGRATION,
    ACTION_RESTART_DAEMON,
    ACTION_OPEN_ACCESSIBILITY,
    ACTION_OPEN_ACCOUNTS,
    ACTION_OPEN_REPOS,
];
pub const ITEM_IDS: [&str; 9] = ["daemon", "hooks", "credentials", "git", "keychain", "ssh", "bindings", "macos", "accessibility"];

/// Git release with `merge-tree --merge-base`, used by the cherry-pick preview.
pub const MIN_GIT: (u32, u32) = (2, 40);
pub const MIN_MACOS_MAJOR: u32 = 14;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticItem {
    pub id: String,
    pub label: String,
    pub state: DiagnosticState,
    pub detail: String,
    pub code: Option<String>,
    pub action: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostics {
    pub generated_at: String,
    pub app_version: String,
    pub macos_version: Option<String>,
    pub git_version: Option<String>,
    pub items: Vec<DiagnosticItem>,
}

impl Diagnostics {
    pub fn count(&self, state: DiagnosticState) -> usize {
        self.items.iter().filter(|i| i.state == state).count()
    }
}

fn item(id: &str, label: &str, state: DiagnosticState, detail: impl Into<String>) -> DiagnosticItem {
    DiagnosticItem { id: id.into(), label: label.into(), state, detail: detail.into(), code: None, action: None }
}

impl DiagnosticItem {
    fn code(mut self, c: &str) -> Self {
        self.code = Some(c.into());
        self
    }

    fn action(mut self, a: &str) -> Self {
        self.action = Some(a.into());
        self
    }
}

/// Keychain probe: `Ok(true)` when the account has a saved access, `Ok(false)` when not.
pub type TokenProbe<'a> = &'a dyn Fn(Uuid) -> Result<bool, String>;

pub struct DiagnosticsInput<'a> {
    pub env: &'a IntegrationEnv,
    pub app_version: &'a str,
    /// `None` skips the item (the CLI cannot tell whether the app has the permission).
    pub accessibility_granted: Option<bool>,
    /// `None` only checks that the Keychain is there (reading items from the CLI may prompt).
    pub token_probe: Option<TokenProbe<'a>>,
}

pub fn run(input: &DiagnosticsInput) -> Diagnostics {
    let status = integration::status(input.env);
    let cfg = AppConfig::load_or_default();
    let macos_version = macos_version();
    let git_version = git_version();

    let mut items = integration_items(&status);
    items.push(git_item(git_version.as_deref()));
    items.push(keychain_item(cfg.as_ref().ok(), input.token_probe));
    items.push(ssh_item(input.env.home.as_deref(), cfg.as_ref().ok()));
    items.push(match &cfg {
        Ok(c) => bindings_item(c),
        Err(e) => item("bindings", "Repositórios vinculados", DiagnosticState::Error, format!("Não foi possível ler as configurações do GitPass ({e}).")).code("GP-CONFIG"),
    });
    items.push(macos_item(macos_version.as_deref()));
    if let Some(granted) = input.accessibility_granted {
        items.push(accessibility_item(granted));
    }

    Diagnostics {
        generated_at: now_rfc3339(),
        app_version: input.app_version.to_string(),
        macos_version,
        git_version,
        items,
    }
}

fn component_state(c: &ComponentStatus) -> DiagnosticState {
    match c.state {
        CheckState::Ok => DiagnosticState::Ok,
        CheckState::Missing | CheckState::Outdated | CheckState::Conflict => DiagnosticState::Warning,
        CheckState::Error => DiagnosticState::Error,
    }
}

fn integration_items(s: &IntegrationStatus) -> Vec<DiagnosticItem> {
    const DAEMON: &str = "Serviço em segundo plano";
    const HOOKS: &str = "Verificação antes de commit e push";
    const CREDENTIALS: &str = "Credenciais do GitHub";
    if !s.enabled {
        let off = "A integração com o Git está desativada. Escolha «Ativar integração» para voltar a conferir as contas.";
        return [("daemon", DAEMON), ("hooks", HOOKS), ("credentials", CREDENTIALS)]
            .into_iter()
            .map(|(id, label)| item(id, label, DiagnosticState::Warning, off).code("GP-INTEGRATION-OFF").action(ACTION_REPAIR_INTEGRATION))
            .collect();
    }
    let binaries_ok = s.binaries.state == CheckState::Ok;
    // The daemon and the hooks run the binaries GitPass copies to Application Support.
    let with_binaries = |id: &str, label: &str, c: &ComponentStatus, code: &str, action: &str| {
        let mut detail = c.summary.clone();
        let mut state = component_state(c);
        if !binaries_ok {
            detail.push(' ');
            detail.push_str(&s.binaries.summary);
            let b = component_state(&s.binaries);
            if b == DiagnosticState::Error || state == DiagnosticState::Ok {
                state = b;
            }
        }
        let it = item(id, label, state, detail);
        match (c.state, state) {
            (_, DiagnosticState::Ok) => it,
            (CheckState::Ok, _) => it.code("GP-INTEGRATION-OUTDATED").action(ACTION_REPAIR_INTEGRATION),
            _ => it.code(code).action(action),
        }
    };

    let daemon_action = if s.daemon.state == CheckState::Error { ACTION_RESTART_DAEMON } else { ACTION_REPAIR_INTEGRATION };
    let daemon = with_binaries("daemon", DAEMON, &s.daemon, "GP-DAEMON-OFF", daemon_action);
    let hooks = with_binaries("hooks", HOOKS, &s.hooks, "GP-HOOKS", ACTION_REPAIR_INTEGRATION);

    let mut credentials = item("credentials", CREDENTIALS, component_state(&s.credentials), s.credentials.summary.clone());
    if s.credentials.state != CheckState::Ok {
        credentials = credentials.code("GP-CREDENTIALS").action(ACTION_REPAIR_INTEGRATION);
    }
    vec![daemon, hooks, credentials]
}

fn command_line(program: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).stdin(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).lines().next().map(|l| l.trim().to_string()).filter(|l| !l.is_empty())
}

pub fn macos_version() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    command_line(Path::new("/usr/bin/sw_vers"), &["-productVersion"])
}

/// `2.50.1` from `git version 2.50.1 (Apple Git-155)`.
pub fn parse_git_version(line: &str) -> Option<String> {
    let v = line.trim().strip_prefix("git version ")?.split_whitespace().next()?;
    v.chars().next().filter(char::is_ascii_digit).map(|_| v.to_string())
}

pub fn git_version() -> Option<String> {
    let git = integration::find_git()?;
    command_line(&git, &["--version"]).and_then(|l| parse_git_version(&l))
}

fn numbers(v: &str) -> Vec<u32> {
    v.split(['.', '-']).map_while(|p| p.parse().ok()).collect()
}

pub fn git_item(version: Option<&str>) -> DiagnosticItem {
    let Some(v) = version else {
        return item(
            "git",
            "Git",
            DiagnosticState::Error,
            "Git não encontrado. Instale as Ferramentas de Linha de Comando do Xcode (no Terminal: xcode-select --install) ou o Git.",
        )
        .code("GP-GIT");
    };
    let n = numbers(v);
    let (major, minor) = (n.first().copied().unwrap_or(0), n.get(1).copied().unwrap_or(0));
    if (major, minor) < MIN_GIT {
        return item(
            "git",
            "Git",
            DiagnosticState::Warning,
            format!(
                "Git {v} é antigo. Commits e push funcionam, mas a prévia de levar commits para outro branch precisa do Git {}.{} ou mais novo.",
                MIN_GIT.0, MIN_GIT.1
            ),
        )
        .code("GP-GIT-OLD");
    }
    item("git", "Git", DiagnosticState::Ok, format!("Git {v}."))
}

fn default_keychain() -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("As Chaves do macOS só existem no macOS.".into());
    }
    let line = command_line(Path::new("/usr/bin/security"), &["default-keychain"])
        .ok_or_else(|| "O macOS não informou as Chaves padrão.".to_string())?;
    let path = line.trim().trim_matches('"');
    if Path::new(path).exists() {
        Ok(())
    } else {
        Err("O arquivo das Chaves padrão não foi encontrado.".into())
    }
}

fn keychain_item(cfg: Option<&AppConfig>, probe: Option<TokenProbe>) -> DiagnosticItem {
    const LABEL: &str = "Chaves do macOS (Keychain)";
    if let Err(e) = default_keychain() {
        return item("keychain", LABEL, DiagnosticState::Error, format!("Não foi possível acessar as Chaves do macOS. {e}")).code("GP-KEYCHAIN");
    }
    let (Some(cfg), Some(probe)) = (cfg, probe) else {
        return item("keychain", LABEL, DiagnosticState::Ok, "As Chaves do macOS estão disponíveis.");
    };
    let mut missing = Vec::new();
    for a in &cfg.accounts {
        let name = a.github_login.as_deref().map(|l| format!("@{l}")).unwrap_or_else(|| a.label.clone().unwrap_or_else(|| "sem nome".into()));
        match probe(a.id) {
            Ok(true) => {}
            Ok(false) => missing.push(name),
            Err(e) => {
                return item("keychain", LABEL, DiagnosticState::Error, format!("Não foi possível ler as Chaves do macOS ({e}). Abra o app Acesso às Chaves e confira se as Chaves estão desbloqueadas."))
                    .code("GP-KEYCHAIN");
            }
        }
    }
    if missing.is_empty() {
        let n = cfg.accounts.len();
        let detail = match n {
            0 => "As Chaves do macOS estão disponíveis. Nenhuma conta conectada ainda.".to_string(),
            1 => "O acesso da conta está salvo nas Chaves do macOS.".to_string(),
            _ => format!("Os acessos das {n} contas estão salvos nas Chaves do macOS."),
        };
        return item("keychain", LABEL, DiagnosticState::Ok, detail);
    }
    item(
        "keychain",
        LABEL,
        DiagnosticState::Warning,
        format!("Sem acesso salvo para {}. Conecte de novo em Contas.", missing.join(", ")),
    )
    .code("GP-OAUTH")
    .action(ACTION_OPEN_ACCOUNTS)
}

/// Files in `~/.ssh` that are never private keys.
const SSH_PUBLIC_FILES: [&str; 6] = ["known_hosts", "known_hosts.old", "config", "authorized_keys", "authorized_keys2", "environment"];

#[cfg(unix)]
fn mode_of(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn mode_of(_meta: &fs::Metadata) -> u32 {
    0o600
}

/// `~/.ssh` permissions: the folder 700 and private keys readable only by you (600 or 400).
/// Only names and permissions are read, never file contents.
pub fn ssh_item(home: Option<&Path>, cfg: Option<&AppConfig>) -> DiagnosticItem {
    const LABEL: &str = "Chaves SSH";
    let uses_ssh = cfg.is_some_and(|c| c.accounts.iter().any(|a| matches!(a.auth, AuthMode::Ssh)));
    let Some(dir) = home.map(|h| h.join(".ssh")) else {
        return item("ssh", LABEL, DiagnosticState::Info, "Não foi possível localizar a pasta pessoal.");
    };
    let Ok(meta) = fs::symlink_metadata(&dir) else {
        let state = if uses_ssh { DiagnosticState::Warning } else { DiagnosticState::Info };
        return item("ssh", LABEL, state, "Não há a pasta ~/.ssh. Ela só é necessária para contas que usam SSH.");
    };
    if !meta.is_dir() {
        return item("ssh", LABEL, DiagnosticState::Warning, "~/.ssh não é uma pasta.").code("GP-SSH-PERMS");
    }
    let mut problems = Vec::new();
    let dir_mode = mode_of(&meta);
    if dir_mode & 0o077 != 0 {
        problems.push(format!("a pasta ~/.ssh está com {dir_mode:o} (o certo é 700: chmod 700 ~/.ssh)"));
    }
    let mut keys = 0;
    let mut open = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.ends_with(".pub") || SSH_PUBLIC_FILES.contains(&name.as_str()) {
                continue;
            }
            let Ok(m) = e.metadata() else { continue };
            if !m.is_file() {
                continue;
            }
            keys += 1;
            if mode_of(&m) & 0o077 != 0 {
                open.push(name);
            }
        }
    }
    if !open.is_empty() {
        open.sort();
        let shown: Vec<String> = open.iter().take(5).map(|n| format!("~/.ssh/{n}")).collect();
        problems.push(format!(
            "outras pessoas do Mac podem ler {} (o certo é 600: chmod 600 {})",
            shown.join(", "),
            shown.join(" ")
        ));
    }
    if problems.is_empty() {
        let detail = match keys {
            0 => "A pasta ~/.ssh está protegida.".to_string(),
            1 => "A pasta ~/.ssh e a chave estão protegidas.".to_string(),
            n => format!("A pasta ~/.ssh e as {n} chaves estão protegidas."),
        };
        return item("ssh", LABEL, DiagnosticState::Ok, detail);
    }
    let mut detail = problems.join("; ");
    if let Some(f) = detail.get_mut(0..1) {
        f.make_ascii_uppercase();
    }
    item("ssh", LABEL, DiagnosticState::Warning, format!("{detail}.")).code("GP-SSH-PERMS")
}

pub fn bindings_item(cfg: &AppConfig) -> DiagnosticItem {
    const LABEL: &str = "Repositórios vinculados";
    if cfg.repos.is_empty() {
        return item("bindings", LABEL, DiagnosticState::Info, "Nenhum repositório vinculado ainda.").action(ACTION_OPEN_REPOS);
    }
    let problems: Vec<(&str, String)> = cfg
        .repos
        .iter()
        .filter_map(|b| {
            let (code, why) = binding_health(cfg, b)?;
            let name = b.root_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| b.root_path.display().to_string());
            Some((code, format!("{name} ({why})")))
        })
        .collect();
    let n = cfg.repos.len();
    if problems.is_empty() {
        let detail = if n == 1 { "O repositório vinculado está em ordem.".to_string() } else { format!("Os {n} repositórios vinculados estão em ordem.") };
        return item("bindings", LABEL, DiagnosticState::Ok, detail);
    }
    let mut shown: Vec<String> = problems.iter().take(5).map(|(_, s)| s.clone()).collect();
    if problems.len() > 5 {
        shown.push(format!("e mais {}", problems.len() - 5));
    }
    let head = if problems.len() == 1 { "1 repositório precisa de atenção".to_string() } else { format!("{} repositórios precisam de atenção", problems.len()) };
    let code = if problems.iter().all(|(c, _)| *c == problems[0].0) { problems[0].0 } else { "GP-BINDING" };
    item("bindings", LABEL, DiagnosticState::Warning, format!("{head}: {}. Confira em Repositórios.", shown.join("; ")))
        .code(code)
        .action(ACTION_OPEN_REPOS)
}

pub fn macos_item(version: Option<&str>) -> DiagnosticItem {
    const LABEL: &str = "macOS";
    let Some(v) = version else {
        let detail = if cfg!(target_os = "macos") {
            "Não foi possível ler a versão do macOS."
        } else {
            "O GitPass foi feito para o macOS 14 (Sonoma) ou mais novo."
        };
        return item("macos", LABEL, DiagnosticState::Warning, detail).code("GP-MACOS");
    };
    if numbers(v).first().copied().unwrap_or(0) < MIN_MACOS_MAJOR {
        return item("macos", LABEL, DiagnosticState::Warning, format!("macOS {v}. O GitPass precisa do macOS 14 (Sonoma) ou mais novo; algumas partes podem não funcionar.")).code("GP-MACOS");
    }
    item("macos", LABEL, DiagnosticState::Ok, format!("macOS {v}."))
}

pub fn accessibility_item(granted: bool) -> DiagnosticItem {
    const LABEL: &str = "Acessibilidade";
    if granted {
        return item("accessibility", LABEL, DiagnosticState::Info, "Permitida: a barra de menus mostra o repositório do app que está na frente (Terminal, VS Code, Cursor…).");
    }
    item(
        "accessibility",
        LABEL,
        DiagnosticState::Info,
        "Não permitida (é opcional). Sem ela, a barra de menus mostra o repositório fixado ou o último usado. Para permitir, abra Ajustes do Sistema › Privacidade e Segurança › Acessibilidade.",
    )
    .action(ACTION_OPEN_ACCESSIBILITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_versions_are_parsed_and_old_ones_flagged() {
        assert_eq!(parse_git_version("git version 2.50.1 (Apple Git-155)").as_deref(), Some("2.50.1"));
        assert_eq!(parse_git_version("git version 2.39.5"), Some("2.39.5".into()));
        assert_eq!(parse_git_version("nope"), None);
        assert_eq!(git_item(Some("2.50.1")).state, DiagnosticState::Ok);
        assert_eq!(git_item(Some("2.40.0")).state, DiagnosticState::Ok);
        let old = git_item(Some("2.39.5"));
        assert_eq!((old.state, old.code.as_deref()), (DiagnosticState::Warning, Some("GP-GIT-OLD")));
        assert!(old.detail.contains("2.40"), "{}", old.detail);
        assert_eq!(git_item(None).state, DiagnosticState::Error);
    }

    #[test]
    fn macos_below_14_is_a_warning() {
        assert_eq!(macos_item(Some("14.0")).state, DiagnosticState::Ok);
        assert_eq!(macos_item(Some("27.0.1")).state, DiagnosticState::Ok);
        let old = macos_item(Some("13.6.1"));
        assert_eq!((old.state, old.code.as_deref()), (DiagnosticState::Warning, Some("GP-MACOS")));
    }

    #[test]
    fn accessibility_is_informational() {
        assert_eq!(accessibility_item(true).state, DiagnosticState::Info);
        assert_eq!(accessibility_item(true).action, None);
        assert_eq!(accessibility_item(false).action.as_deref(), Some(ACTION_OPEN_ACCESSIBILITY));
    }

    #[cfg(unix)]
    #[test]
    fn ssh_permissions_are_checked_without_reading_keys() {
        use std::os::unix::fs::PermissionsExt;
        let home = std::env::temp_dir().join(format!("gitpass-ssh-{}", Uuid::new_v4()));
        let ssh = home.join(".ssh");
        fs::create_dir_all(&ssh).unwrap();
        let set = |p: &Path, m: u32| fs::set_permissions(p, fs::Permissions::from_mode(m)).unwrap();
        set(&ssh, 0o700);
        for (name, mode) in [("id_ed25519", 0o600), ("id_ed25519.pub", 0o644), ("config", 0o644), ("known_hosts", 0o644)] {
            fs::write(ssh.join(name), "x").unwrap();
            set(&ssh.join(name), mode);
        }
        let ok = ssh_item(Some(&home), None);
        assert_eq!(ok.state, DiagnosticState::Ok, "{}", ok.detail);

        set(&ssh.join("id_ed25519"), 0o644);
        set(&ssh, 0o755);
        let bad = ssh_item(Some(&home), None);
        assert_eq!((bad.state, bad.code.as_deref()), (DiagnosticState::Warning, Some("GP-SSH-PERMS")));
        assert!(bad.detail.contains("chmod 700 ~/.ssh") && bad.detail.contains("chmod 600 ~/.ssh/id_ed25519"), "{}", bad.detail);
        assert!(!bad.detail.contains(&home.display().to_string()));

        let none = ssh_item(Some(&home.join("nobody")), None);
        assert_eq!(none.state, DiagnosticState::Info);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn missing_folders_are_listed_in_bindings() {
        let account_id = Uuid::new_v4();
        let cfg = AppConfig {
            accounts: vec![crate::Account { id: account_id, github_login: Some("octo".into()), ..Default::default() }],
            repos: vec![crate::RepoBinding {
                id: Uuid::new_v4(),
                account_id,
                root_path: std::env::temp_dir().join(format!("gitpass-gone-{}", Uuid::new_v4())),
                github_owner: Some("octo".into()),
                github_repo: Some("app".into()),
            }],
            ..Default::default()
        };
        let it = bindings_item(&cfg);
        assert_eq!((it.state, it.code.as_deref(), it.action.as_deref()), (DiagnosticState::Warning, Some("GP-REPO"), Some(ACTION_OPEN_REPOS)));
        assert!(it.detail.contains("a pasta não existe mais"), "{}", it.detail);
        assert_eq!(bindings_item(&AppConfig::default()).state, DiagnosticState::Info);
    }

    #[test]
    fn integration_items_point_to_repair() {
        let c = |state| ComponentStatus { state, summary: "x".into(), current_value: None, previous_value: None };
        let s = IntegrationStatus {
            enabled: true,
            git_found: true,
            binaries: c(CheckState::Ok),
            credentials: c(CheckState::Conflict),
            hooks: c(CheckState::Ok),
            daemon: c(CheckState::Error),
            has_backup: false,
            ready: false,
        };
        let items = integration_items(&s);
        assert_eq!(items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["daemon", "hooks", "credentials"]);
        assert_eq!((items[0].state, items[0].action.as_deref()), (DiagnosticState::Error, Some(ACTION_RESTART_DAEMON)));
        assert_eq!((items[1].state, items[1].code.as_deref()), (DiagnosticState::Ok, None));
        assert_eq!(items[2].action.as_deref(), Some(ACTION_REPAIR_INTEGRATION));
        let stale = integration_items(&IntegrationStatus { binaries: c(CheckState::Outdated), daemon: c(CheckState::Ok), ..s.clone() });
        assert_eq!((stale[0].state, stale[0].code.as_deref()), (DiagnosticState::Warning, Some("GP-INTEGRATION-OUTDATED")));
        assert_eq!((stale[1].state, stale[1].action.as_deref()), (DiagnosticState::Warning, Some(ACTION_REPAIR_INTEGRATION)));
        let off = integration_items(&IntegrationStatus { enabled: false, ..s });
        assert!(off.iter().all(|i| i.state == DiagnosticState::Warning && i.action.as_deref() == Some(ACTION_REPAIR_INTEGRATION)));
    }
}
