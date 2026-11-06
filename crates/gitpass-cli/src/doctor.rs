//! `gitpass doctor` — environment checks, using the same integration status as the app.

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command;

use gitpass_core::integration::{self, CheckState, ComponentStatus, IntegrationEnv, IntegrationStatus};
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

    if cfg!(target_os = "macos") {
        match Command::new("/usr/bin/sw_vers").arg("-productVersion").output() {
            Ok(out) if out.status.success() => {
                let s = String::from_utf8_lossy(&out.stdout);
                match s.lines().next().map(str::trim).filter(|v| !v.is_empty()) {
                    Some(ver) => r.ok(format!("macOS {ver}")),
                    None => r.warn("não foi possível ler a versão do macOS"),
                }
            }
            _ => r.warn("não foi possível rodar o sw_vers"),
        }
    } else {
        r.warn("o GitPass foi feito para o macOS 14 ou mais novo");
    }

    match integration::find_git() {
        Some(git) => match Command::new(&git).arg("--version").output() {
            Ok(out) if out.status.success() => {
                let s = String::from_utf8_lossy(&out.stdout);
                r.ok(format!("{} ({})", s.lines().next().unwrap_or("git").trim(), git.display()));
            }
            _ => r.err(format!("o Git em {} não respondeu", git.display())),
        },
        None => r.err("Git não encontrado (instale as Xcode Command Line Tools ou o Git)"),
    }

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
            let status = integration::status(&env);
            if !status.enabled {
                r.warn("integração desativada (foi desfeita); para ativar de novo: gitpass hooks install");
            }
            for (label, c) in components(&status) {
                let line = format!("{label}: {}", component_detail(c));
                match c.state {
                    CheckState::Ok => r.ok(line),
                    CheckState::Error => r.err(line),
                    CheckState::Missing | CheckState::Outdated | CheckState::Conflict => r.warn(line),
                }
            }
            if !status.ready && status.enabled {
                r.lines.push("Para reparar: gitpass hooks install (ou «Reparar» em Configurações no app GitPass).".into());
            }
        }
        Err(e) => r.err(format!("pasta de dados do GitPass: {e}")),
    }

    r.ok(format!("este binário: {}", current_exe().display()));
    r.print_and_status()
}
