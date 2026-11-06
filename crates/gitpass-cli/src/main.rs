//! `gitpass` CLI — hooks, doctor, IPC to `gitpassd`.

mod doctor;

use std::io::{BufRead, BufReader, Write};
use std::str::FromStr;
#[cfg(unix)]
use std::os::unix::net::UnixStream as StdUnixStream;
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use doctor::{integration_env, print_integration_status, run as doctor_run};
use gitpass_core::integration::{self, ApplyOptions, CheckState, IntegrationReport};
use gitpass_core::{
    ci_bypass_from_env, config_file_path, decode_response_line, encode_line, git_remote_url,
    git_work_tree_root,
    github_oauth_device_login, github_oauth_env_credentials, github_oauth_login, github_token_delete,
    github_token_save, resolve_oauth_client_id,
    ipc_socket_path, normalize_owner, parse_github_remote,
    policy_when_daemon_unreachable, validate_repo_for_hook, Account, AppConfig,
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
    /// Check Git, the GitPass integration (credentials, hooks, daemon) and the config.
    Doctor,
    /// Install or undo the Git integration (same as Definições › Integração com o Git in the app).
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
                "O serviço gitpassd é gerido pela app GitPass (Definições › Integração com o Git). Para depurar, corre o binário gitpassd diretamente."
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

fn update_last_active_repo(cwd: &std::path::Path) -> Result<(), std::io::Error> {
    let mut cfg = AppConfig::load_or_default().map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
    })?;
    let bid = match binding_id_for_path(&cfg, cwd) {
        Ok(id) => id,
        Err(_) => return Ok(()),
    };
    if cfg.ui.last_active_repo_binding_id == Some(bid) {
        return Ok(());
    }
    cfg.ui.last_active_repo_binding_id = Some(bid);
    cfg.save().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    Ok(())
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
            cfg.save()?;
            println!("ok: pinned repo binding {bid} (menu bar tooltip)");
            println!("config: {}", config_file_path()?.display());
            Ok(0)
        }
        UiCmd::Unpin => {
            let mut cfg = AppConfig::load_or_default()?;
            cfg.ui.pinned_repo_binding_id = None;
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
    let code = print_report(&report, "a integração com o Git já estava em vigor.");
    if report.status.hooks.state == CheckState::Conflict {
        println!("\nPara encadear os teus hooks globais (o GitPass valida primeiro e depois corre os teus): gitpass hooks install --chain");
    }
    Ok(code)
}

fn integration_undo() -> Result<i32, Box<dyn std::error::Error + Send + Sync>> {
    let env = integration_env()?;
    let report = integration::undo(&env)?;
    let code = print_report(&report, "não havia nada a desfazer.");
    println!(
        "\nA app GitPass não volta a ativar a integração sozinha. Para a reativar: gitpass hooks install ou «Ativar integração» em Definições."
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

fn run_hook(phase: &str) -> Result<i32, Box<dyn std::error::Error + Send + Sync>> {
    HookPhase::from_str(phase)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

    if ci_bypass_from_env() {
        let quiet = std::env::var("GITPASS_QUIET_HOOK").ok().as_deref() == Some("1");
        if !quiet {
            eprintln!("GitPass: ambiente de CI autorizado — verificações ignoradas no hook {phase}.");
        }
        return Ok(0);
    }

    let cwd = std::env::current_dir()?;
    let ctx = HookContext {
        phase: phase.to_string(),
        cwd,
    };

    let quiet = std::env::var("GITPASS_QUIET_HOOK").ok().as_deref() == Some("1");

    #[cfg(unix)]
    {
        let sock = ipc_socket_path()?;
        if sock.exists() {
            match ipc_validate_hook_sync(&ctx) {
                Ok(v) => {
                    let code = handle_validate_outcome(v, quiet);
                    if code == 0 {
                        let _ = update_last_active_repo(&ctx.cwd);
                    }
                    return Ok(code);
                }
                Err(_) => {
                    if !quiet {
                        eprintln!("GitPass: aviso: o serviço do GitPass não respondeu; a validar sem ele.");
                    }
                }
            }
        }
    }

    // Offline: re-read config in the hook process (may differ until we add SIGHUP reload in daemon).
    let cfg = AppConfig::load_or_default()?;
    if let ValidateFail::Err { code, message } = policy_when_daemon_unreachable(phase, &cfg) {
        if !quiet {
            eprintln!("GitPass [{code}] {message}");
        }
        return Ok(1);
    }
    match validate_repo_for_hook(&ctx, &cfg) {
        ValidateFail::Ok => {
            let _ = update_last_active_repo(&ctx.cwd);
            Ok(0)
        }
        ValidateFail::OkUnconfirmed { warning } => {
            if !quiet {
                eprintln!("{warning}");
            }
            let _ = update_last_active_repo(&ctx.cwd);
            Ok(0)
        }
        ValidateFail::Err { code, message } => {
            if !quiet {
                eprintln!("GitPass [{code}] {message}");
            }
            Ok(1)
        }
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
                    eprintln!("GitPass [GP-IPC] O serviço do GitPass deu uma resposta inválida. Abre a app GitPass e escolhe «Reparar» em Definições.");
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
                eprintln!("GitPass [GP-IPC] Resposta inesperada do serviço do GitPass. Abre a app GitPass e escolhe «Reparar» em Definições.");
            }
            1
        }
    }
}

#[cfg(unix)]
fn ipc_validate_hook_sync(ctx: &HookContext) -> Result<IpcResponse, Box<dyn std::error::Error + Send + Sync>> {
    let path = ipc_socket_path()?;
    let mut stream = StdUnixStream::connect(&path)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let req = IpcRequest::ValidateHook {
        phase: ctx.phase.clone(),
        cwd: ctx.cwd.clone(),
    };
    let line = encode_line(&req)?;
    stream.write_all(line.as_bytes())?;

    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.is_empty() {
        return Err("empty response".into());
    }
    Ok(decode_response_line(&line)?)
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
