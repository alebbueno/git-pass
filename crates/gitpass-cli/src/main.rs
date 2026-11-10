//! `gitpass` CLI — hooks, doctor, IPC to `gitpassd`.

mod doctor;
mod recover;

use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use doctor::{integration_env, print_integration_status, run as doctor_run};
use gitpass_core::audit::{self, AuditKind};
use gitpass_core::integration::{self, ApplyOptions, CheckState, IntegrationReport};
use gitpass_core::recent::{self, EmergencyVia, HookOutcome, Verdict};
use gitpass_core::{
    app_support_dir, ci_bypass_from_env, config_file_path, decode_response_line, emergency, encode_line,
    git_remote_url, git_work_tree_root,
    github_oauth_device_login, github_oauth_env_credentials, github_oauth_login, github_token_delete,
    github_token_save, resolve_oauth_client_id,
    ipc_socket_path, normalize_owner, parse_github_remote,
    ipc_client, is_validated_phase, offline_policy, validate_repo_for_hook,
    Account, AppConfig, DaemonUnreachable, IpcTimeouts, OfflineAllowance,
    AuthMode as CoreAuthMode,
    DEFAULT_GITHUB_SCOPES, DEFAULT_OAUTH_CALLBACK_PORT, KEYCHAIN_SERVICE,
    HookContext, HookPhase, IpcRequest, IpcResponse, RepoBinding, ValidateFail,
};
use uuid::Uuid;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as AsyncBufReader};
use tokio::net::UnixStream as AsyncUnixStream;
use tokio::time::timeout;

#[derive(Parser)]
#[command(name = "gitpass", version, about = "GitPass — GitHub account guard (v1)")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage GitHub accounts (stored in config.toml).
    Account {
        #[command(subcommand)]
        sub: AccountCmd,
    },
    /// Bind local repositories to accounts.
    Repo {
        #[command(subcommand)]
        sub: RepoCmd,
    },
    /// Git global config helpers (HTTPS credentials).
    Config {
        #[command(subcommand)]
        sub: ConfigCmd,
    },
    /// Tray / UI hints (`config.toml` `[ui]` section).
    Ui {
        #[command(subcommand)]
        sub: UiCmd,
    },
    /// Emergency mode: allow local commits for a while when the GitPass service does not answer (pushes stay blocked).
    Emergency {
        #[command(subcommand)]
        sub: EmergencyCmd,
    },
    /// Check Git, the GitPass integration (credentials, hooks, daemon) and the config.
    Doctor,
    /// Install or undo the Git integration (same as Configurações › Integração com o Git in the app).
    Hooks(HooksCmd),
    /// Internal: invoked by hook scripts (validates repo against config.toml).
    Hook(HookCmd),
    /// Check connectivity to the background daemon.
    Ping,
    /// Run the daemon in the foreground (same as `gitpassd`, for debugging).
    Daemon,
}

#[derive(Parser)]
struct HooksCmd {
    #[command(subcommand)]
    sub: HooksSub,
}

#[derive(Subcommand)]
enum HooksSub {
    /// Install or repair the whole integration: hooks, GitHub credential helper and gitpassd.
    Install {
        /// Replace an existing global `core.hooksPath`; your hooks keep running after GitPass's.
        #[arg(long)]
        chain: bool,
    },
    /// Undo the integration and restore the previous Git configuration.
    Uninstall,
}

#[derive(Parser)]
struct HookCmd {
    #[arg(long)]
    phase: String,
}

#[derive(Subcommand)]
enum AccountCmd {
    /// Add a new account (UUID generated).
    Add(AccountAddArgs),
    /// Sign in with GitHub (OAuth in browser). Stores token in Keychain. Needs `GITHUB_OAUTH_CLIENT_ID` / `GITHUB_OAUTH_CLIENT_SECRET` and a matching callback URL on the OAuth app.
    Login(AccountLoginArgs),
    /// List accounts.
    List,
    /// Remove an account by id (repos bound to it are left; run `gitpass repo list` / `unbind`).
    Remove {
        #[arg(long)]
        id: Uuid,
    },
    /// Rebuild accounts in config.toml from Keychain OAuth tokens (`tokens/*.present`).
    Recover(RecoverArgs),
}

#[derive(Args)]
struct RecoverArgs {
    /// Only print what would change.
    #[arg(long)]
    dry_run: bool,
    /// Scan this folder for Git repos whose local `user.email` matches a recovered account (repeatable).
    #[arg(long, value_name = "PATH")]
    scan: Vec<PathBuf>,
}

#[derive(Args)]
struct AccountAddArgs {
    /// GitHub username (login) for this account.
    #[arg(long)]
    login: String,
    #[arg(long)]
    label: Option<String>,
    #[arg(long, value_enum, default_value_t = CliAuth::Ssh)]
    auth: CliAuth,
}

#[derive(Args)]
struct AccountLoginArgs {
    /// Create a new account from the GitHub login returned by OAuth (default: refresh token for `--account`).
    #[arg(long)]
    new: bool,
    /// Existing account **id** or **login** to store/refresh the token (not used with `--new`).
    #[arg(long)]
    account: Option<String>,
    /// Local port for `http://127.0.0.1:{port}/callback` — must match the OAuth app settings exactly.
    #[arg(long, default_value_t = DEFAULT_OAUTH_CALLBACK_PORT)]
    port: u16,
    /// Space-separated OAuth scopes (default: read:user read:org repo).
    #[arg(long)]
    scopes: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum CliAuth {
    #[default]
    Ssh,
    Https,
}

#[derive(Subcommand)]
enum RepoCmd {
    /// Bind the current (or given) repository work tree to an account. Parses `origin` from Git.
    Bind(RepoBindArgs),
    /// List repo bindings.
    List,
    /// Remove the binding for a path.
    Unbind {
        /// Directory inside the repo (default: current directory).
        #[arg(long)]
        path: Option<std::path::PathBuf>,
    },
}

#[derive(Args)]
struct RepoBindArgs {
    /// Account id (UUID) **or** GitHub login of an existing account.
    #[arg(long)]
    account: String,
    /// Path inside the repo (default: current directory).
    #[arg(long)]
    path: Option<std::path::PathBuf>,
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Install or repair the integration (includes `git-credential-gitpass` for `https://github.com`).
    InstallCredentialHelper,
    /// Undo the integration (same as `gitpass hooks uninstall`).
    UninstallCredentialHelper,
}

#[derive(Subcommand)]
enum EmergencyCmd {
    /// Turn emergency mode on.
    On {
        /// Duration in minutes (5 to 240).
        #[arg(long, default_value_t = 30)]
        minutes: i64,
        /// Why (3 to 200 characters); kept in the local audit log.
        #[arg(long)]
        reason: String,
    },
    /// Turn emergency mode off.
    Off,
    /// Show whether emergency mode is on.
    Status,
}

#[derive(Subcommand)]
enum UiCmd {
    /// Pin the current repo binding for the menu bar tooltip (see GitPass desktop app).
    Pin {
        #[arg(long)]
        path: Option<std::path::PathBuf>,
    },
    /// Clear the pinned repo (`ui.pinned_repo_binding_id`).
    Unpin,
}

fn main() {
    let cli = Cli::parse();
    let code = match run(cli) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    };
    std::process::exit(code);
}

fn run(cli: Cli) -> Result<i32, Box<dyn std::error::Error + Send + Sync>> {
    match cli.command {
        Commands::Account { sub } => account_cmd(sub),
        Commands::Repo { sub } => repo_cmd(sub),
        Commands::Config { sub } => match sub {
            ConfigCmd::InstallCredentialHelper => integration_apply(false),
            ConfigCmd::UninstallCredentialHelper => integration_undo(),
        },
        Commands::Ui { sub } => ui_cmd(sub),
        Commands::Emergency { sub } => emergency_cmd(sub),
        Commands::Doctor => doctor_run().map_err(|e| e.into()),
        Commands::Hooks(h) => match h.sub {
            HooksSub::Install { chain } => integration_apply(chain),
            HooksSub::Uninstall => integration_undo(),
        },
        Commands::Hook(h) => run_hook(&h.phase),
        Commands::Ping => {
            // Keep async ping on a small runtime
            Ok(tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(ping_async())?)
        }
        Commands::Daemon => {
            eprintln!(
                "O serviço em segundo plano é gerenciado pelo app GitPass (Configurações › Integração com o Git). Para depurar, rode o binário gitpassd diretamente."
            );
            Ok(0)
        }
    }
}

fn account_cmd(cmd: AccountCmd) -> Result<i32, Box<dyn std::error::Error + Send + Sync>> {
    match cmd {
        AccountCmd::Add(args) => {
            let mut cfg = AppConfig::load_or_default()?;
            let login = normalize_owner(&args.login);
            if find_account_by_login(&cfg, &login).is_some() {
                eprintln!("account with login `{login}` already exists");
                return Ok(1);
            }
            let id = Uuid::new_v4();
            let auth = match args.auth {
                CliAuth::Ssh => CoreAuthMode::Ssh,
                CliAuth::Https => CoreAuthMode::Https,
            };
            cfg.accounts.push(Account {
                id,
                github_login: Some(login.clone()),
                label: args.label.clone(),
                auth,
                ..Default::default()
            });
            cfg.save()?;
            println!("ok: added account {id} ({login})");
            println!("config: {}", config_file_path()?.display());
            Ok(0)
        }
        AccountCmd::Login(args) => {
            if args.new && args.account.is_some() {
                eprintln!("use either --new or --account, not both");
                return Ok(1);
            }
            if !args.new && args.account.is_none() {
                eprintln!("pass --new to create an account from GitHub login, or --account <id|login> to refresh");
                return Ok(1);
            }

            let scopes = args
                .scopes
                .clone()
                .or_else(|| std::env::var("GITPASS_OAUTH_SCOPES").ok())
                .unwrap_or_else(|| DEFAULT_GITHUB_SCOPES.to_string());

            let mut cfg = AppConfig::load_or_default()?;

            let (bundle, login_remote) =
                if let Ok((client_id, client_secret)) = github_oauth_env_credentials() {
                    let port = std::env::var("GITPASS_OAUTH_PORT")
                        .ok()
                        .and_then(|s| s.parse::<u16>().ok())
                        .unwrap_or(args.port);
                    println!(
                        "Starting OAuth (browser + localhost) — callback URL must be exactly http://127.0.0.1:{port}/callback\n\
                         (set this as \"Authorization callback URL\" on your GitHub OAuth app).\n"
                    );
                    github_oauth_login(&client_id, &client_secret, port, &scopes).map_err(|e| {
                        std::io::Error::new(std::io::ErrorKind::Other, format!("{e}"))
                    })?
                } else {
                    let client_id = resolve_oauth_client_id().map_err(|e| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            format!(
                                "{e} Or set GITHUB_OAUTH_CLIENT_ID and GITHUB_OAUTH_CLIENT_SECRET for the web flow."
                            ),
                        )
                    })?;
                    println!(
                        "Starting OAuth device flow (no client secret) — enable Device flow on your GitHub OAuth app.\n"
                    );
                    github_oauth_device_login(&client_id, &scopes).map_err(|e| {
                        std::io::Error::new(std::io::ErrorKind::Other, format!("{e}"))
                    })?
                };
            let login_norm = normalize_owner(&login_remote);

            if args.new {
                if find_account_by_login(&cfg, &login_norm).is_some() {
                    eprintln!("account `{login_remote}` already exists — use `gitpass account login --account {login_remote}` to refresh");
                    return Ok(1);
                }
                let id = Uuid::new_v4();
                cfg.accounts.push(Account {
                    id,
                    github_login: Some(login_remote.clone()),
                    label: None,
                    auth: CoreAuthMode::Https,
                    ..Default::default()
                });
                github_token_save(id, &bundle)?;
                cfg.save()?;
                println!(
                    "ok: signed in as {login_remote}, account {id}\n\
                     token stored in Keychain (service {KEYCHAIN_SERVICE})",
                );
                println!("config: {}", config_file_path()?.display());
                return Ok(0);
            }

            let acc_s = args.account.as_ref().unwrap();
            let account_id = resolve_account_id(&cfg, acc_s)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

            let acc = cfg
                .accounts
                .iter_mut()
                .find(|a| a.id == account_id)
                .expect("resolved id must exist");

            if let Some(existing) = acc.github_login.as_deref() {
                if normalize_owner(existing) != login_norm {
                    eprintln!(
                        "GitHub login `{login_remote}` does not match account's github_login `{existing}`"
                    );
                    return Ok(1);
                }
            } else {
                acc.github_login = Some(login_remote.clone());
            }

            github_token_save(account_id, &bundle)?;
            cfg.save()?;

            println!("ok: refreshed OAuth token for account {account_id} ({login_remote})");
            println!("config: {}", config_file_path()?.display());
            Ok(0)
        }
        AccountCmd::List => {
            let cfg = AppConfig::load_or_default()?;
            if cfg.accounts.is_empty() {
                println!("(no accounts — use `gitpass account add`)");
                return Ok(0);
            }
            for a in &cfg.accounts {
                let login = a.github_login.as_deref().unwrap_or("(no login)");
                let label = a
                    .label
                    .as_deref()
                    .map(|l| format!(" ({l})"))
                    .unwrap_or_default();
                println!(
                    "{} {}{} — {:?}",
                    a.id, login, label, a.auth
                );
            }
            Ok(0)
        }
        AccountCmd::Remove { id } => {
            let mut cfg = AppConfig::load_or_default()?;
            let pos = cfg.accounts.iter().position(|a| a.id == id);
            let Some(i) = pos else {
                eprintln!("no account with id {id}");
                return Ok(1);
            };
            let _ = github_token_delete(id);
            cfg.accounts.remove(i);
            cfg.save()?;
            println!("ok: removed account {id}");
            Ok(0)
        }
        AccountCmd::Recover(args) => {
            let mut scan = args.scan;
            if scan.is_empty() {
                scan.push(PathBuf::from("/Volumes/Dados/Server"));
            }
            match recover::run_recover(&scan, args.dry_run) {
                Ok(r) => {
                    println!(
                        "contas: {}, repos (recent/scan): {}/{}, tokens ignorados: {}",
                        r.accounts, r.repos_from_recent, r.repos_from_scan, r.skipped_tokens
                    );
                    Ok(0)
                }
                Err(e) => {
                    eprintln!("{e}");
                    Ok(1)
                }
            }
        }
    }
}

fn find_account_by_login<'a>(cfg: &'a AppConfig, login: &str) -> Option<&'a Account> {
    let n = normalize_owner(login);
    cfg.accounts
        .iter()
        .find(|a| a.github_login.as_ref().map(|l| normalize_owner(l) == n).unwrap_or(false))
}

fn resolve_account_id(cfg: &AppConfig, account: &str) -> Result<Uuid, String> {
    if let Ok(u) = Uuid::parse_str(account) {
        if cfg.accounts.iter().any(|a| a.id == u) {
            return Ok(u);
        }
        return Err(format!("no account with id {u}"));
    }
    let login = normalize_owner(account);
    find_account_by_login(cfg, &login)
        .map(|a| a.id)
        .ok_or_else(|| format!("no account with login `{account}`"))
}

/// Resolve git work tree root and matching `RepoBinding` id for `config.toml` updates.
fn binding_id_for_path(
    cfg: &AppConfig,
    cwd: &std::path::Path,
) -> Result<Uuid, std::io::Error> {
    let root = git_work_tree_root(Some(cwd)).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;
    let root = root.canonicalize()?;
    let id = cfg
        .repos
        .iter()
        .find(|r| {
            r.root_path
                .canonicalize()
                .unwrap_or_else(|_| r.root_path.clone())
                == root
        })
        .map(|r| r.id)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no GitPass binding for repo root {}", root.display()),
            )
        })?;
    Ok(id)
}

fn print_emergency(v: &emergency::EmergencyView) {
    match (&v.until, &v.reason) {
        (Some(until), reason) if v.active => {
            println!("Modo emergência ativo até {until}. Commits locais passam mesmo sem o serviço do GitPass; o push continua bloqueado.");
            if let Some(r) = reason {
                println!("Motivo: {r}");
            }
        }
        _ => println!("Modo emergência desligado."),
    }
}

fn emergency_cmd(cmd: EmergencyCmd) -> Result<i32, Box<dyn std::error::Error + Send + Sync>> {
    let result = match cmd {
        EmergencyCmd::On { minutes, reason } => emergency::start_now(minutes, &reason, "cli"),
        EmergencyCmd::Off => emergency::stop_now("cli"),
        EmergencyCmd::Status => emergency::expire_now().and_then(|_| emergency::view_now()),
    };
    match result {
        Ok(v) => {
            print_emergency(&v);
            Ok(0)
        }
        Err(e) => {
            eprintln!("{e}");
            Ok(1)
        }
    }
}

fn ui_cmd(cmd: UiCmd) -> Result<i32, Box<dyn std::error::Error + Send + Sync>> {
    match cmd {
        UiCmd::Pin { path } => {
            let mut cfg = AppConfig::load_or_default()?;
            let cwd = path.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let bid = binding_id_for_path(&cfg, &cwd).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("{e}. Run `gitpass repo bind` in this repository first."),
                )
            })?;
            cfg.ui.pinned_repo_binding_id = Some(bid);
            cfg.ui.pinned_at = Some(audit::now_rfc3339());
            cfg.save()?;
            println!("ok: pinned repo binding {bid} (menu bar tooltip)");
            println!("config: {}", config_file_path()?.display());
            Ok(0)
        }
        UiCmd::Unpin => {
            let mut cfg = AppConfig::load_or_default()?;
            cfg.ui.pinned_repo_binding_id = None;
            cfg.ui.pinned_at = None;
            cfg.save()?;
            println!("ok: cleared pinned repo");
            println!("config: {}", config_file_path()?.display());
            Ok(0)
        }
    }
}

fn print_report(r: &IntegrationReport, nothing_changed: &str) -> i32 {
    for c in &r.changes {
        println!("ok: {c}");
    }
    if r.changes.is_empty() {
        println!("ok: {nothing_changed}");
    }
    for w in &r.warnings {
        eprintln!("aviso: {w}");
    }
    println!();
    print_integration_status(&r.status);
    if r.warnings.is_empty() {
        0
    } else {
        2
    }
}

/// Same as "Reparar" in the app: copies this build's binaries to Application Support and points
/// Git (credentials, `core.hooksPath`) and launchd at those copies.
fn integration_apply(chain: bool) -> Result<i32, Box<dyn std::error::Error + Send + Sync>> {
    let env = integration_env()?;
    let report = integration::apply(&env, ApplyOptions { take_over_hooks: chain })?;
    let code = print_report(&report, "a integração com o Git já estava ativa.");
    if report.status.hooks.state == CheckState::Conflict {
        println!("\nPara manter as suas verificações globais do Git (o GitPass confere a conta primeiro e depois roda as suas): gitpass hooks install --chain");
    }
    Ok(code)
}

fn integration_undo() -> Result<i32, Box<dyn std::error::Error + Send + Sync>> {
    let env = integration_env()?;
    let report = integration::undo(&env)?;
    let code = print_report(&report, "não havia nada para desfazer.");
    println!(
        "\nO app GitPass não ativa a integração de novo sozinho. Para ativar: gitpass hooks install ou «Ativar integração» em Configurações."
    );
    Ok(code)
}

fn repo_cmd(cmd: RepoCmd) -> Result<i32, Box<dyn std::error::Error + Send + Sync>> {
    match cmd {
        RepoCmd::Bind(args) => {
            let mut cfg = AppConfig::load_or_default()?;
            let account_id = resolve_account_id(&cfg, &args.account)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

            let cwd = args
                .path
                .clone()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let root = git_work_tree_root(Some(&cwd))
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
            let root = root
                .canonicalize()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

            let origin_url = git_remote_url(&root, "origin")
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
            let gh = parse_github_remote(&origin_url).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("origin is not a GitHub URL we understand: {origin_url}"),
                )
            })?;

            let idx = cfg.repos.iter().position(|r| {
                r.root_path
                    .canonicalize()
                    .unwrap_or_else(|_| r.root_path.clone())
                    == root
            });

            let id = if let Some(i) = idx {
                let existing = &mut cfg.repos[i];
                existing.account_id = account_id;
                existing.github_owner = Some(gh.owner.clone());
                existing.github_repo = Some(gh.repo.clone());
                existing.id
            } else {
                let new_id = Uuid::new_v4();
                cfg.repos.push(RepoBinding {
                    id: new_id,
                    account_id,
                    root_path: root.clone(),
                    github_owner: Some(gh.owner.clone()),
                    github_repo: Some(gh.repo.clone()),
                });
                new_id
            };

            cfg.save()?;
            println!("ok: bound {} → account {account_id} (binding {id})", root.display());
            println!("  origin: {origin_url}");
            println!("config: {}", config_file_path()?.display());
            Ok(0)
        }
        RepoCmd::List => {
            let cfg = AppConfig::load_or_default()?;
            if cfg.repos.is_empty() {
                println!("(no bindings — use `gitpass repo bind`)");
                return Ok(0);
            }
            for r in &cfg.repos {
                let owner = r
                    .github_owner
                    .as_deref()
                    .map(|o| format!("{o}/"))
                    .unwrap_or_default();
                let repo = r.github_repo.as_deref().unwrap_or("?");
                println!(
                    "{}  {}  → account {}  ({owner}{repo})",
                    r.id,
                    r.root_path.display(),
                    r.account_id
                );
            }
            Ok(0)
        }
        RepoCmd::Unbind { path } => {
            let mut cfg = AppConfig::load_or_default()?;
            let cwd = path.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let root = git_work_tree_root(Some(&cwd))
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
            let root = root
                .canonicalize()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

            let pos = cfg.repos.iter().position(|r| {
                r.root_path
                    .canonicalize()
                    .unwrap_or_else(|_| r.root_path.clone())
                    == root
            });
            let Some(i) = pos else {
                eprintln!("no binding for {}", root.display());
                return Ok(1);
            };
            cfg.repos.remove(i);
            cfg.save()?;
            println!("ok: removed binding for {}", root.display());
            Ok(0)
        }
    }
}

/// Saves the verdict for the menu bar (`recent.json`) and the audit log. Never fails the hook.
fn note_outcome(cwd: &std::path::Path, cfg: Option<&AppConfig>, outcome: &HookOutcome) {
    let Ok(dir) = app_support_dir() else { return };
    let Ok(root) = git_work_tree_root(Some(cwd)) else { return };
    let root = root.canonicalize().unwrap_or(root);
    let loaded;
    let cfg = match cfg {
        Some(c) => Some(c),
        None => {
            loaded = AppConfig::load_or_default().ok();
            loaded.as_ref()
        }
    };
    let _ = recent::note_hook_outcome_in(&dir, cfg, &root, outcome);
}

fn note_ci_bypass(cwd: &std::path::Path, phase: &str) {
    let Ok(dir) = app_support_dir() else { return };
    let root = git_work_tree_root(Some(cwd)).ok();
    let cfg = AppConfig::load_or_default().ok();
    let repo = root.as_deref().map(|r| {
        let name = cfg.as_ref().and_then(|c| recent::binding_for_root(c, r)).and_then(recent::full_name);
        recent::repo_label(name.as_deref(), r)
    });
    let _ = audit::append_in(
        &dir,
        AuditKind::CiBypass,
        repo.as_deref(),
        None,
        &format!("Verificações ignoradas em ambiente de CI autorizado ({phase})."),
    );
}

fn blocked(code: &str, message: String) -> HookOutcome {
    HookOutcome { verdict: Verdict::Blocked, code: Some(code.into()), message: Some(message), emergency: None }
}

fn run_hook(phase: &str) -> Result<i32, Box<dyn std::error::Error + Send + Sync>> {
    HookPhase::from_str(phase)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

    let quiet = std::env::var("GITPASS_QUIET_HOOK").ok().as_deref() == Some("1");
    let validated = is_validated_phase(phase);
    let cwd = std::env::current_dir()?;

    if ci_bypass_from_env() {
        if !quiet {
            eprintln!("GitPass: ambiente de CI autorizado — verificações ignoradas em {phase}.");
        }
        if validated {
            note_ci_bypass(&cwd, phase);
        }
        return Ok(0);
    }

    let ctx = HookContext {
        phase: phase.to_string(),
        cwd,
    };

    #[cfg(unix)]
    let unreachable = match ipc_validate_hook_sync(&ctx) {
        Ok(v) => {
            let outcome = outcome_of_response(&v);
            let code = handle_validate_outcome(v, quiet);
            if validated {
                note_outcome(&ctx.cwd, None, &outcome);
            }
            return Ok(code);
        }
        Err(reason) => reason,
    };
    #[cfg(not(unix))]
    let unreachable = DaemonUnreachable::NotRunning;

    // Offline: re-read config in the hook process (may differ until we add SIGHUP reload in daemon).
    let cfg = AppConfig::load_or_default()?;
    let allowance = match offline_policy(phase, &cfg, unreachable) {
        Ok(a) => a,
        Err(ValidateFail::Err { code, mut message }) => {
            if phase == "pre-push" && emergency::is_active(&cfg) {
                message.push_str(" O modo emergência libera só commits locais; o push continua bloqueado.");
            }
            if !quiet {
                eprintln!("GitPass [{code}] {message}");
            }
            note_outcome(&ctx.cwd, Some(&cfg), &blocked(code, message));
            return Ok(1);
        }
        Err(_) => return Ok(1),
    };
    if !validated {
        return Ok(0);
    }
    let emergency = match allowance {
        OfflineAllowance::Emergency => Some(EmergencyVia::Toggle),
        OfflineAllowance::EmergencyEnv => Some(EmergencyVia::Env),
        OfflineAllowance::NotValidated | OfflineAllowance::PushOverride => None,
    };
    if !quiet {
        if emergency.is_some() {
            eprintln!("GitPass: aviso: o serviço do GitPass não respondeu. O modo emergência está ativo: o commit local é conferido só aqui no Mac, e o push continua bloqueado.");
        } else {
            eprintln!("GitPass: aviso: o serviço do GitPass não respondeu; conferindo sem ele.");
        }
    }
    let (exit, outcome) = match validate_repo_for_hook(&ctx, &cfg) {
        ValidateFail::Ok => (0, HookOutcome { verdict: Verdict::Ok, code: None, message: None, emergency }),
        ValidateFail::OkUnconfirmed { warning } => {
            if !quiet {
                eprintln!("{warning}");
            }
            (0, HookOutcome { verdict: Verdict::Unconfirmed, code: None, message: Some(warning), emergency })
        }
        // Confirming an org/collaborator repository needs the account's access, which only the
        // service reads; in an emergency the commit goes on unconfirmed (the push is checked again).
        ValidateFail::Err { code: "GP-OAUTH", .. } if emergency.is_some() => {
            let warning = "GitPass: aviso: sem o serviço do GitPass não deu para confirmar a conta deste repositório no GitHub. O commit local foi liberado pelo modo emergência; a conta será conferida de novo no push.".to_string();
            if !quiet {
                eprintln!("{warning}");
            }
            (0, HookOutcome { verdict: Verdict::Unconfirmed, code: Some("GP-OAUTH".into()), message: Some(warning), emergency })
        }
        ValidateFail::Err { code, message } => {
            if !quiet {
                eprintln!("GitPass [{code}] {message}");
            }
            (1, blocked(code, message))
        }
    };
    note_outcome(&ctx.cwd, Some(&cfg), &outcome);
    Ok(exit)
}

fn outcome_of_response(v: &IpcResponse) -> HookOutcome {
    match v {
        IpcResponse::ValidateResult { ok: true, unconfirmed, warning, .. } => HookOutcome {
            verdict: if *unconfirmed { Verdict::Unconfirmed } else { Verdict::Ok },
            code: None,
            message: warning.clone().filter(|_| *unconfirmed),
            emergency: None,
        },
        IpcResponse::ValidateResult { ok: false, code, message, .. } => blocked(
            code.as_deref().unwrap_or("GP-IPC"),
            message.clone().unwrap_or_else(|| "O serviço do GitPass enviou uma resposta inválida.".into()),
        ),
        IpcResponse::Error { code, message } => blocked(code, message.clone()),
        IpcResponse::Pong { .. } => blocked("GP-IPC", "O serviço do GitPass enviou uma resposta que não entendemos.".into()),
    }
}

fn handle_validate_outcome(v: IpcResponse, quiet: bool) -> i32 {
    match v {
        IpcResponse::ValidateResult {
            ok,
            unconfirmed,
            code,
            message,
            warning,
        } => {
            if ok {
                if unconfirmed {
                    if let Some(w) = warning {
                        if !quiet {
                            eprintln!("{w}");
                        }
                    }
                }
                0
            } else if let (Some(code), Some(msg)) = (code, message) {
                if !quiet {
                    eprintln!("GitPass [{code}] {msg}");
                }
                1
            } else {
                if !quiet {
                    eprintln!("GitPass [GP-IPC] O serviço do GitPass enviou uma resposta inválida. Abra o app GitPass e escolha «Reparar» em Configurações.");
                }
                1
            }
        }
        IpcResponse::Error { code, message } => {
            if !quiet {
                eprintln!("GitPass [{code}] {message}");
            }
            1
        }
        _ => {
            if !quiet {
                eprintln!("GitPass [GP-IPC] O serviço do GitPass enviou uma resposta que não entendemos. Abra o app GitPass e escolha «Reparar» em Configurações.");
            }
            1
        }
    }
}

/// Asks the daemon, waiting through restarts and slow GitHub answers (see `gitpass_core::ipc_client`).
#[cfg(unix)]
fn ipc_validate_hook_sync(ctx: &HookContext) -> Result<IpcResponse, DaemonUnreachable> {
    let path = ipc_socket_path().map_err(|_| DaemonUnreachable::NotRunning)?;
    let req = IpcRequest::ValidateHook {
        phase: ctx.phase.clone(),
        cwd: ctx.cwd.clone(),
    };
    let mut timeouts = IpcTimeouts::default();
    // Debug builds only (integration tests): do not wait for a daemon that is never started.
    #[cfg(debug_assertions)]
    if let Some(ms) = std::env::var("GITPASS_TEST_IPC_CONNECT_WAIT_MS").ok().and_then(|v| v.parse().ok()) {
        timeouts.connect_wait = Duration::from_millis(ms);
    }
    if !is_validated_phase(&ctx.phase) {
        // Nothing to block in these phases: don't make Git wait for a daemon that is off.
        timeouts.connect_wait = Duration::ZERO;
    }
    // Debug builds only (integration tests without a daemon). Release builds always wait.
    #[cfg(debug_assertions)]
    if let Some(ms) = std::env::var("GITPASS_TEST_IPC_CONNECT_WAIT_MS").ok().and_then(|v| v.parse().ok()) {
        timeouts.connect_wait = Duration::from_millis(ms);
    }
    ipc_client::request(&path, &req, timeouts)
}

async fn ping_async() -> Result<i32, Box<dyn std::error::Error + Send + Sync>> {
    let path = ipc_socket_path()?;
    if !path.exists() {
        eprintln!(
            "gitpassd does not appear to be running (no socket at {}).",
            path.display()
        );
        return Ok(1);
    }

    let mut stream = AsyncUnixStream::connect(&path).await?;
    let req = encode_line(&IpcRequest::Ping)?;
    stream.write_all(req.as_bytes()).await?;

    let mut reader = AsyncBufReader::new(stream);
    let mut line = String::new();
    let read_fut = reader.read_line(&mut line);
    match timeout(Duration::from_secs(5), read_fut).await {
        Ok(Ok(0)) => {
            eprintln!("empty response from gitpassd");
            return Ok(1);
        }
        Ok(Ok(_n)) => {}
        Ok(Err(e)) => {
            eprintln!("read error: {e}");
            return Ok(1);
        }
        Err(_) => {
            eprintln!("timeout waiting for gitpassd");
            return Ok(1);
        }
    }

    match decode_response_line(&line)? {
        IpcResponse::Pong { gitpassd_version } => {
            println!("ok — gitpassd {}", gitpassd_version);
            Ok(0)
        }
        IpcResponse::Error { code, message } => {
            eprintln!("error {code}: {message}");
            Ok(1)
        }
        _ => {
            eprintln!("unexpected response from gitpassd");
            Ok(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(ok: bool, unconfirmed: bool, code: Option<&str>, message: Option<&str>, warning: Option<&str>) -> IpcResponse {
        IpcResponse::ValidateResult {
            ok,
            unconfirmed,
            code: code.map(String::from),
            message: message.map(String::from),
            warning: warning.map(String::from),
        }
    }

    #[test]
    fn daemon_replies_become_hook_verdicts() {
        let ok = outcome_of_response(&result(true, false, None, None, None));
        assert_eq!((ok.verdict, ok.code.as_deref(), ok.message.as_deref()), (Verdict::Ok, None, None));
        let warn = outcome_of_response(&result(true, true, None, None, Some("GitPass: aviso: x")));
        assert_eq!((warn.verdict, warn.message.as_deref()), (Verdict::Unconfirmed, Some("GitPass: aviso: x")));
        let no = outcome_of_response(&result(false, false, Some("GP-OWNER"), Some("m"), None));
        assert_eq!((no.verdict, no.code.as_deref(), no.message.as_deref()), (Verdict::Blocked, Some("GP-OWNER"), Some("m")));
        let err = outcome_of_response(&IpcResponse::Error { code: "GP-IPC-DECODE".into(), message: "x".into() });
        assert_eq!((err.verdict, err.code.as_deref()), (Verdict::Blocked, Some("GP-IPC-DECODE")));
        assert!([ok, warn, no, err].iter().all(|o| o.emergency.is_none()));
    }
}
