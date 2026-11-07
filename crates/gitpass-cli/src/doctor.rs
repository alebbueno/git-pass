//! `gitpass doctor` — environment checks, using the same integration status as the app.

use std::io::{self, Write};
use std::path::PathBuf;

use gitpass_core::diagnostics::{self, DiagnosticState, DiagnosticsInput, ACTION_REPAIR_INTEGRATION, ACTION_RESTART_DAEMON};
use gitpass_core::integration::{CheckState, ComponentStatus, IntegrationEnv, IntegrationStatus};
use gitpass_core::{config_file_path, AppConfig};

#[derive(Default)]
pub struct DoctorReport {
    pub lines: Vec<String>,
    pub errors: usize,
    pub warnings: usize,
}

impl DoctorReport {
    fn ok(&mut self, msg: impl Into<String>) {
        self.lines.push(format!("ok: {}", msg.into()));
    }

    fn warn(&mut self, msg: impl Into<String>) {
        self.warnings += 1;
        self.lines.push(format!("aviso: {}", msg.into()));
    }

    fn info(&mut self, msg: impl Into<String>) {
        self.lines.push(format!("info: {}", msg.into()));
    }

    fn err(&mut self, msg: impl Into<String>) {
        self.errors += 1;
        self.lines.push(format!("erro: {}", msg.into()));
    }

    pub fn print_and_status(&self) -> io::Result<i32> {
        let mut out = io::stdout().lock();
        for line in &self.lines {
            writeln!(out, "{line}")?;
        }
        let code = if self.errors > 0 {
            1
        } else if self.warnings > 0 {
            2
        } else {
            0
        };
        Ok(code)
    }
}

/// Path to this `gitpass` executable.
pub fn current_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("gitpass"))
}

/// The real machine, with the binaries next to this executable as the source for `apply`
/// (`GitPass.app/Contents/MacOS` or `target/<profile>`).
pub fn integration_env() -> io::Result<IntegrationEnv> {
    let exe = current_exe();
    let exe = exe.canonicalize().unwrap_or(exe);
    IntegrationEnv::system(exe.parent().map(PathBuf::from))
}

fn state_label(s: CheckState) -> &'static str {
    match s {
        CheckState::Ok => "ok",
        CheckState::Missing => "faltando",
        CheckState::Outdated => "desatualizado",
        CheckState::Conflict => "em conflito",
        CheckState::Error => "erro",
    }
}

fn components(s: &IntegrationStatus) -> [(&'static str, &ComponentStatus); 4] {
    [
        ("Peças da integração", &s.binaries),
        ("Credenciais GitHub", &s.credentials),
        ("Verificações do Git", &s.hooks),
        ("Serviço em segundo plano", &s.daemon),
    ]
}

fn component_detail(c: &ComponentStatus) -> String {
    let mut s = c.summary.clone();
    if let Some(v) = &c.current_value {
        s.push_str(&format!(" (atual: {v})"));
    }
    if let Some(v) = &c.previous_value {
        s.push_str(&format!(" (anterior: {v})"));
    }
    s
}

pub fn print_integration_status(s: &IntegrationStatus) {
    for (label, c) in components(s) {
        println!("  {label}: {} — {}", state_label(c.state), component_detail(c));
    }
}

pub fn run() -> io::Result<i32> {
    let mut r = DoctorReport::default();

    match config_file_path() {
        Ok(p) => match AppConfig::load_or_default() {
            Ok(cfg) if p.exists() => r.ok(format!(
                "configurações: {} contas, {} repositórios ({})",
                cfg.accounts.len(),
                cfg.repos.len(),
                p.display()
            )),
            Ok(_) => r.warn("ainda não há configurações: abra o app GitPass e conecte uma conta"),
            Err(e) => r.err(format!("configurações inválidas em {}: {e}", p.display())),
        },
        Err(e) => r.err(format!("pasta de dados do GitPass: {e}")),
    }

    match integration_env() {
        Ok(env) => {
            // Same checks as Diagnóstico in the app; the Keychain items are not read here (a
            // different program reading them may make macOS ask for permission).
            let d = diagnostics::run(&DiagnosticsInput {
                env: &env,
                app_version: env!("CARGO_PKG_VERSION"),
                accessibility_granted: None,
                token_probe: None,
            });
            for it in &d.items {
                let code = it.code.as_deref().map(|c| format!(" [{c}]")).unwrap_or_default();
                let line = format!("{}: {}{code}", it.label, it.detail);
                match it.state {
                    DiagnosticState::Ok => r.ok(line),
                    DiagnosticState::Warning => r.warn(line),
                    DiagnosticState::Error => r.err(line),
                    DiagnosticState::Info => r.info(line),
                }
            }
            let repair = [ACTION_REPAIR_INTEGRATION, ACTION_RESTART_DAEMON];
            if d.items.iter().any(|i| i.action.as_deref().is_some_and(|a| repair.contains(&a))) {
                r.lines.push("Para reparar: gitpass hooks install (ou «Reparar» em Configurações no app GitPass).".into());
            }
        }
        Err(e) => r.err(format!("pasta de dados do GitPass: {e}")),
    }

    r.ok(format!("este binário: {}", current_exe().display()));
    r.print_and_status()
}
