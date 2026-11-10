//! On-demand commit / push identity diagnostic for a bound repository (read-only Git inspection).

use std::path::Path;
use std::process::Command;

use uuid::Uuid;

use crate::bindings::{
    expected_identity, identity_state, pick_remote, read_remotes, read_repo_identity, redact_remote_url,
    remote_url_has_credentials, RemotePick, RepoIdentity,
};
use crate::github_api::{token_can_push_for_account, GithubApiError};
use crate::github_user::fetch_user;
use crate::integration::{is_gitpass_helper, CREDENTIAL_HELPER_KEY, CREDENTIAL_USE_HTTP_PATH_KEY};
use crate::parse_github_remote;
use crate::redact::redact_home;
use crate::secrets::github_token_load;
use crate::{Account, AppConfig, RepoBinding};

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct IdentityOrigin {
    pub key: String,
    pub value: Option<String>,
    pub scope: Option<String>,
    /// Config source (file path or special), without credentials.
    pub origin: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct CommitDiagnostic {
    /// `correct` | `different` | `not_set` | `unavailable`
    pub state: String,
    pub summary: String,
    pub detail: Option<String>,
    pub effective_name: Option<String>,
    pub effective_email: Option<String>,
    pub expected_name: Option<String>,
    pub expected_email: Option<String>,
    /// `git var GIT_AUTHOR_IDENT` (name + email only).
    pub author_context_name: Option<String>,
    pub author_context_email: Option<String>,
    pub origins: Vec<IdentityOrigin>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct PushDestination {
    pub remote_name: String,
    pub label: String,
    pub host: Option<String>,
    pub owner: Option<String>,
    pub repo: Option<String>,
    pub schemes: Vec<String>,
    pub is_origin: bool,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct PushDiagnostic {
    /// `ok` | `missing` | `unavailable`
    pub destination_state: String,
    pub destination_summary: String,
    pub selected_remote: Option<String>,
    pub available_remotes: Vec<String>,
    pub destinations: Vec<PushDestination>,
    /// `gitpass_routed` | `unverified` | `ssh` | `embedded_credentials` | `external_helper` | `no_destination`
    pub auth_state: String,
    pub auth_summary: String,
    pub bound_account_login: Option<String>,
    pub token_login: Option<String>,
    pub token_push_permission: Option<bool>,
    pub token_message: Option<String>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct IdentityDiagnostic {
    pub binding_id: String,
    pub exists: bool,
    pub commit: CommitDiagnostic,
    pub push: PushDiagnostic,
}

fn git_stdout(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(root).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn git_global_get_all(key: &str) -> Vec<String> {
    let out = Command::new("git")
        .args(["config", "--global", "--includes", "--get-all", key])
        .output()
        .ok();
    let Some(out) = out else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

fn is_truthy(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "yes" | "on" | "1")
}

fn sanitize_origin(origin: &str) -> String {
    redact_home(origin.trim(), dirs::home_dir().as_deref())
}

fn parse_author_ident(raw: &str) -> (Option<String>, Option<String>) {
    let raw = raw.trim();
    let Some(lt) = raw.rfind('<') else {
        return (None, None);
    };
    let Some(gt_rel) = raw[lt + 1..].find('>') else {
        return (None, None);
    };
    let gt = gt_rel + lt + 1;
    let email = raw[lt + 1..gt].trim();
    let name = raw[..lt].trim();
    (
        (!name.is_empty()).then(|| name.to_string()),
        (!email.is_empty()).then(|| email.to_string()),
    )
}

fn read_git_author_context(root: &Path) -> (Option<String>, Option<String>) {
    git_stdout(root, &["var", "GIT_AUTHOR_IDENT"]).map(|s| parse_author_ident(&s)).unwrap_or((None, None))
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn parse_config_origin_line(line: &str, default_key: &str) -> Option<IdentityOrigin> {
    let line = line.trim_end();
    if line.is_empty() {
        return None;
    }
    let parts: Vec<&str> = line.split('\t').collect();
    match parts.len() {
        4 => Some(IdentityOrigin {
            origin: Some(sanitize_origin(parts[0])),
            scope: non_empty(parts[1]),
            key: parts[2].to_string(),
            value: non_empty(parts[3]),
        }),
        3 => Some(IdentityOrigin {
            origin: Some(sanitize_origin(parts[0])),
            scope: None,
            key: default_key.to_string(),
            value: non_empty(parts[2]),
        }),
        _ => None,
    }
}

fn read_identity_origins(root: &Path) -> Vec<IdentityOrigin> {
    let mut out = Vec::new();
    for key in ["user.name", "user.email"] {
        let args = ["config", "--show-origin", "--show-scope", "--get-all", "--", key];
        if let Some(text) = git_stdout(root, &args) {
            for line in text.lines() {
                if let Some(entry) = parse_config_origin_line(line, key) {
                    out.push(entry);
                }
            }
            continue;
        }
        // Git without --show-scope (older): origin\tvalue
        let args = ["config", "--show-origin", "--get-all", "--", key];
        if let Some(text) = git_stdout(root, &args) {
            for line in text.lines() {
                if let Some(entry) = parse_config_origin_line(line, key) {
                    out.push(entry);
                }
            }
        }
    }
    out
}

fn read_push_urls(root: &Path, remote: &str) -> Vec<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["remote", "get-url", "--push", "--all", remote])
        .output()
        .ok();
    let Some(out) = out else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|u| redact_remote_url(u))
        .collect()
}

fn scheme_of(url: &str) -> &'static str {
    let u = url.trim();
    if u.starts_with("git@") || u.starts_with("ssh://") {
        "ssh"
    } else if u.starts_with("http://") || u.starts_with("https://") {
        "https"
    } else {
        "other"
    }
}

fn format_person(name: Option<&str>, email: Option<&str>) -> String {
    match (name, email) {
        (Some(n), Some(e)) => format!("{n} · {e}"),
        (Some(n), None) => n.to_string(),
        (None, Some(e)) => e.to_string(),
        (None, None) => "não definida".to_string(),
    }
}

fn build_commit_diag(root: &Path, expected: &RepoIdentity, local: &RepoIdentity, exists: bool) -> CommitDiagnostic {
    if !exists {
        return CommitDiagnostic {
            state: "unavailable".into(),
            summary: "Pasta do repositório não encontrada".into(),
            detail: None,
            effective_name: None,
            effective_email: None,
            expected_name: expected.name.clone(),
            expected_email: expected.email.clone(),
            author_context_name: None,
            author_context_email: None,
            origins: Vec::new(),
        };
    }
    let state = identity_state(local, expected);
    let (ctx_name, ctx_email) = read_git_author_context(root);
    let origins = read_identity_origins(root);
    let effective_name = ctx_name.clone().or_else(|| local.name.clone());
    let effective_email = ctx_email.clone().or_else(|| local.email.clone());
    let expected_label = format_person(expected.name.as_deref(), expected.email.as_deref());
    let local_label = format_person(local.name.as_deref(), local.email.as_deref());
    let (state_s, summary, detail) = match state {
        crate::bindings::IdentityState::Correct => (
            "correct",
            format!("Autor alinhado · {local_label}"),
            ctx_name
                .as_ref()
                .zip(ctx_email.as_ref())
                .filter(|(n, e)| local.name.as_deref() != Some(n.as_str()) || local.email.as_deref() != Some(e.as_str()))
                .map(|_| "O contexto efetivo do Git (GIT_AUTHOR_IDENT) pode diferir do user.name/user.email locais.".into()),
        ),
        crate::bindings::IdentityState::Different => (
            "different",
            format!("{local_label} · esperado {expected_label}"),
            Some("A identidade local desta pasta não coincide com a conta vinculada.".into()),
        ),
        crate::bindings::IdentityState::NotSet => (
            "not_set",
            format!("Sem user.name/user.email local · esperado {expected_label}"),
            Some("Os próximos commits podem usar a identidade global do Git ou variáveis de ambiente.".into()),
        ),
    };
    CommitDiagnostic {
        state: state_s.into(),
        summary: summary,
        detail,
        effective_name,
        effective_email,
        expected_name: expected.name.clone(),
        expected_email: expected.email.clone(),
        author_context_name: ctx_name,
        author_context_email: ctx_email,
        origins,
    }
}

fn push_destination_label(urls: &[String]) -> String {
    for u in urls {
        if let Some(id) = parse_github_remote(u) {
            return format!("github.com/{}/{}", id.owner, id.repo);
        }
    }
    urls.first().map(|u| redact_remote_url(u)).unwrap_or_else(|| "destino desconhecido".into())
}

fn build_push_diag(
    root: &Path,
    binding: &RepoBinding,
    account: Option<&Account>,
    remote_pref: Option<&str>,
    verify_token: bool,
    exists: bool,
) -> PushDiagnostic {
    if !exists {
        return PushDiagnostic {
            destination_state: "unavailable".into(),
            destination_summary: "Pasta do repositório não encontrada".into(),
            selected_remote: None,
            available_remotes: Vec::new(),
            destinations: Vec::new(),
            auth_state: "no_destination".into(),
            auth_summary: "Conta de autenticação não verificada".into(),
            bound_account_login: account.and_then(|a| a.github_login.clone()),
            token_login: None,
            token_push_permission: None,
            token_message: None,
            evidence: Vec::new(),
        };
    }
    let remotes = read_remotes(root);
    let names: Vec<String> = remotes.iter().map(|(n, _)| n.clone()).collect();
    let selected = remote_pref
        .filter(|r| names.iter().any(|n| n == *r))
        .map(String::from)
        .or_else(|| match pick_remote(&remotes) {
            RemotePick::GitHub { name, .. } | RemotePick::Bitbucket { name, .. } | RemotePick::Other { name, .. } => Some(name),
            RemotePick::None => None,
        });

    let (destination_state, destination_summary, destinations): (String, String, Vec<PushDestination>) = if names.is_empty() {
        (
            "missing".into(),
            "Nenhum remoto configurado".into(),
            Vec::new(),
        )
    } else if selected.is_none() {
        (
            "ambiguous".into(),
            "Escolha um remoto para analisar".into(),
            Vec::new(),
        )
    } else {
        let remote_name = selected.clone().unwrap();
        let push_urls = read_push_urls(root, &remote_name);
        let fetch_url = remotes.iter().find(|(n, _)| *n == remote_name).map(|(_, u)| redact_remote_url(u));
        let mut urls = push_urls.clone();
        if urls.is_empty() {
            if let Some(ref u) = fetch_url {
                urls.push(u.clone());
            }
        }
        let label = if push_urls.is_empty() && fetch_url.is_some() {
            format!("{} (usa URL de fetch)", push_destination_label(&urls))
        } else {
            push_destination_label(&urls)
        };
        let schemes: Vec<String> = urls.iter().map(|u| scheme_of(u).to_string()).collect();
        let (host, owner, repo) = urls
            .iter()
            .find_map(|u| parse_github_remote(u).map(|id| (Some("github.com".into()), Some(id.owner), Some(id.repo))))
            .unwrap_or((None, None, None));
        let dest = PushDestination {
            remote_name: remote_name.clone(),
            label: label.clone(),
            host,
            owner: owner.clone(),
            repo: repo.clone(),
            schemes: schemes.clone(),
            is_origin: remote_name == "origin",
        };
        ("ok".into(), format!("Remoto `{}` · {label}", remote_name), vec![dest])
    };

    let mut evidence = Vec::new();
    let helpers = git_global_get_all(CREDENTIAL_HELPER_KEY);
    let use_http_path = git_global_get_all(CREDENTIAL_USE_HTTP_PATH_KEY).last().map(|s| is_truthy(s));
    if helpers.iter().any(|h| is_gitpass_helper(h)) {
        evidence.push("Helper de credencial do GitPass presente na configuração global.".into());
    } else if !helpers.is_empty() {
        evidence.push("Outros helpers de credencial para github.com.".into());
    } else {
        evidence.push("Nenhum helper de credencial global para github.com.".into());
    }
    if let Some(true) = use_http_path {
        evidence.push("credential.https://github.com.useHttpPath está ativo.".into());
    }

    let push_urls = selected.as_ref().map(|r| read_push_urls(root, r)).unwrap_or_default();
    let auth_state: String;
    let auth_summary: String;
    if destination_state != "ok" {
        auth_state = "no_destination".into();
        auth_summary = "Conta de autenticação não verificada".into();
    } else if push_urls.iter().any(|u| remote_url_has_credentials(u)) {
        auth_state = "embedded_credentials".into();
        auth_summary = "Conta de autenticação não verificada".into();
        evidence.push("A URL de push contém credenciais embutidas.".into());
    } else if push_urls.iter().all(|u| scheme_of(u) == "ssh") {
        auth_state = "ssh".into();
        auth_summary = "Conta de autenticação não verificada".into();
        evidence.push("O push usa SSH; o GitPass não inspeciona a chave do agente.".into());
    } else if helpers.iter().any(|h| is_gitpass_helper(h)) {
        let login = account.and_then(|a| a.github_login.clone());
        auth_state = "gitpass_routed".into();
        auth_summary = login
            .map(|l| format!("Conta selecionada pelo GitPass · @{l}"))
            .unwrap_or_else(|| "Conta selecionada pelo GitPass".into());
    } else if !helpers.is_empty() {
        auth_state = "external_helper".into();
        auth_summary = "Conta de autenticação não verificada".into();
        evidence.push("Outro helper de credencial pode atender o push.".into());
    } else {
        auth_state = "unverified".into();
        auth_summary = "Conta de autenticação não verificada".into();
    }

    let (token_login, token_push_permission, token_message) = if verify_token {
        match account {
            None => (None, None, Some("Conta vinculada ausente.".into())),
            Some(acc) => {
                let owner = destinations
                    .first()
                    .and_then(|d| d.owner.clone())
                    .or_else(|| binding.github_owner.clone());
                let repo = destinations
                    .first()
                    .and_then(|d| d.repo.clone())
                    .or_else(|| binding.github_repo.clone());
                let token_res = github_token_load(acc.id);
                match token_res {
                    Err(e) => (None, None, Some(format!("Não foi possível ler o token ({e})."))),
                    Ok(None) => (None, None, Some("Nenhum token no Keychain.".into())),
                    Ok(Some(bundle)) => {
                        let user = fetch_user(&bundle.access_token);
                        let login = user.as_ref().ok().map(|u| u.login.clone());
                        let perm = match (&owner, &repo) {
                            (Some(o), Some(r)) => {
                                token_can_push_for_account(acc.id, o, r).ok()
                            }
                            _ => None,
                        };
                        let msg = match &user {
                            Ok(_) => perm.map(|p| {
                                if p {
                                    "Permissão de push reportada para o token do perfil.".into()
                                } else {
                                    "O token do perfil não tem permissão de push neste repositório.".into()
                                }
                            }),
                            Err(GithubApiError::Http { status, .. }) if *status == 401 => {
                                Some("Token expirado ou revogado.".into())
                            }
                            Err(e) => Some(format!("Não foi possível consultar o GitHub ({e}).")),
                        };
                        (login, perm, msg)
                    }
                }
            }
        }
    } else {
        (None, None, None)
    };

    PushDiagnostic {
        destination_state,
        destination_summary,
        selected_remote: selected,
        available_remotes: names,
        destinations,
        auth_state,
        auth_summary,
        bound_account_login: account.and_then(|a| a.github_login.clone()),
        token_login,
        token_push_permission,
        token_message,
        evidence,
    }
}

/// Read-only diagnostic for a bound repo (`binding_id`). Does not mutate Git or config.
pub fn binding_identity_diagnostic(
    cfg: &AppConfig,
    binding_id: Uuid,
    remote: Option<&str>,
    verify_token: bool,
) -> Result<IdentityDiagnostic, String> {
    let binding = cfg
        .repos
        .iter()
        .find(|r| r.id == binding_id)
        .cloned()
        .ok_or_else(|| "Vínculo desconhecido.".to_string())?;
    let account = cfg.accounts.iter().find(|a| a.id == binding.account_id);
    let root = &binding.root_path;
    let exists = root.is_dir();
    let expected = account.map(expected_identity).unwrap_or_default();
    let local = if exists { read_repo_identity(root) } else { RepoIdentity::default() };
    let commit = build_commit_diag(root, &expected, &local, exists);
    let push = build_push_diag(root, &binding, account, remote, verify_token, exists);
    Ok(IdentityDiagnostic {
        binding_id: binding_id.to_string(),
        exists,
        commit,
        push,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::apply_repo_identity;
    use crate::config::Account;
    use crate::RepoBinding;
    use std::fs;
    use std::process::Command;
    use uuid::Uuid;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gitpass-id-diag-{name}-{}", Uuid::new_v4()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn init_repo(dir: &Path, remote: &str) {
        Command::new("git")
            .current_dir(dir)
            .args(["init", "-q"])
            .status()
            .unwrap();
        Command::new("git")
            .current_dir(dir)
            .args(["config", "user.email", "dev@example.com"])
            .status()
            .unwrap();
        Command::new("git")
            .current_dir(dir)
            .args(["config", "user.name", "Dev"])
            .status()
            .unwrap();
        fs::write(dir.join("README.md"), "x").unwrap();
        Command::new("git").current_dir(dir).args(["add", "README.md"]).status().unwrap();
        Command::new("git")
            .current_dir(dir)
            .args(["commit", "-m", "init"])
            .status()
            .unwrap();
        Command::new("git")
            .current_dir(dir)
            .args(["remote", "add", "origin", remote])
            .status()
            .unwrap();
    }

    #[test]
    fn author_ident_is_parsed() {
        let (n, e) = parse_author_ident("Ana Costa <ana@example.com> 1760000000 +0000");
        assert_eq!(n.as_deref(), Some("Ana Costa"));
        assert_eq!(e.as_deref(), Some("ana@example.com"));
    }

    #[test]
    fn commit_different_when_local_differs() {
        let path = temp_dir("commit");
        init_repo(&path, "https://github.com/octo/hello.git");
        let mut cfg = AppConfig::default();
        let acc = Account {
            id: Uuid::new_v4(),
            github_login: Some("octo".into()),
            name: Some("Octo".into()),
            email: Some("octo@users.noreply.github.com".into()),
            ..Default::default()
        };
        cfg.accounts.push(acc.clone());
        let canon = path.canonicalize().unwrap();
        let binding = RepoBinding {
            id: Uuid::new_v4(),
            account_id: acc.id,
            root_path: canon,
            github_owner: Some("octo".into()),
            github_repo: Some("hello".into()),
        };
        cfg.repos.push(binding.clone());
        apply_repo_identity(&path, &acc).unwrap();
        Command::new("git")
            .current_dir(&path)
            .args(["config", "--local", "user.email", "other@example.com"])
            .status()
            .unwrap();
        let d = binding_identity_diagnostic(&cfg, binding.id, None, false).unwrap();
        let _ = fs::remove_dir_all(&path);
        assert_eq!(d.commit.state, "different");
        assert!(d.commit.summary.contains("esperado"));
        assert_eq!(d.push.destination_state, "ok");
    }

    #[test]
    fn ssh_push_is_unverified() {
        let path = temp_dir("ssh");
        init_repo(&path, "git@github.com:octo/hello.git");
        let mut cfg = AppConfig::default();
        let acc = Account {
            id: Uuid::new_v4(),
            github_login: Some("octo".into()),
            email: Some("o@x.pt".into()),
            ..Default::default()
        };
        cfg.accounts.push(acc.clone());
        let canon = path.canonicalize().unwrap();
        let binding = RepoBinding {
            id: Uuid::new_v4(),
            account_id: acc.id,
            root_path: canon,
            github_owner: Some("octo".into()),
            github_repo: Some("hello".into()),
        };
        cfg.repos.push(binding.clone());
        apply_repo_identity(&path, &acc).unwrap();
        let d = binding_identity_diagnostic(&cfg, binding.id, None, false).unwrap();
        assert_eq!(d.push.auth_state, "ssh");
        assert!(d.push.auth_summary.contains("não verificada"));
        let _ = fs::remove_dir_all(&path);
    }
}
