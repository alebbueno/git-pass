//! "Exportar logs" (DL-035): a ZIP with the daemon log, the audit log, `recent.json`, the
//! diagnostics and a `manifest.json` (versions, date, files with SHA-256). Every file is redacted
//! (tokens, credentials, private keys) and the home folder is written as `~`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::audit::{logs_dir, now_rfc3339};
use crate::diagnostics::Diagnostics;
use crate::recent::recent_file;
use crate::redact::redact_for_export;

/// Larger logs keep only their end.
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManifestFile {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Manifest {
    pub generated_at: String,
    pub app_version: String,
    pub macos_version: Option<String>,
    pub git_version: Option<String>,
    pub redacted: bool,
    pub files: Vec<ManifestFile>,
}

/// `GitPass-logs-AAAA-MM-DD.zip` for the local date.
pub fn default_file_name(now: chrono::DateTime<chrono::Local>) -> String {
    format!("GitPass-logs-{}.zip", now.format("%Y-%m-%d"))
}

fn read_tail(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let start = bytes.len().saturating_sub(MAX_FILE_BYTES as usize);
    let mut text = String::from_utf8_lossy(&bytes[start..]).into_owned();
    if start > 0 {
        // Drop the partial first line (and anything a cut could have left half-redacted).
        text = text.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default();
    }
    Some(text)
}

/// Text files that go into the archive, as `(name in the ZIP, path)`.
fn sources(app_support: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let logs = logs_dir(app_support);
    if let Ok(entries) = fs::read_dir(&logs) {
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.') && (n.ends_with(".log") || n.ends_with(".jsonl")))
            .collect();
        names.sort();
        out.extend(names.into_iter().map(|n| (format!("logs/{n}"), logs.join(&n))));
    }
    let recent = recent_file(app_support);
    if recent.is_file() {
        out.push(("recent.json".into(), recent));
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes the archive to `dest` (atomically) and returns its manifest.
pub fn write_logs_zip(
    app_support: &Path,
    home: Option<&Path>,
    diagnostics: &Diagnostics,
    dest: &Path,
) -> Result<Manifest, String> {
    let mut files: Vec<(String, Vec<u8>)> = sources(app_support)
        .into_iter()
        .filter_map(|(name, path)| read_tail(&path).map(|t| (name, redact_for_export(&t, home).into_bytes())))
        .collect();
    let diag = serde_json::to_string_pretty(diagnostics).map_err(|e| e.to_string())?;
    files.push(("diagnostics.json".into(), redact_for_export(&diag, home).into_bytes()));

    let manifest = Manifest {
        generated_at: now_rfc3339(),
        app_version: diagnostics.app_version.clone(),
        macos_version: diagnostics.macos_version.clone(),
        git_version: diagnostics.git_version.clone(),
        redacted: true,
        files: files
            .iter()
            .map(|(name, bytes)| ManifestFile { name: name.clone(), bytes: bytes.len() as u64, sha256: sha256_hex(bytes) })
            .collect(),
    };
    let manifest_json = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;

    let parent = dest.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let fail = |e: &dyn std::fmt::Display| format!("Não foi possível salvar o arquivo ({e}).");
    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        std::process::id()
    ));
    let write = || -> Result<(), String> {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let file = opts.open(&tmp).map_err(|e| fail(&e))?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o600);
        for (name, bytes) in files.iter().map(|(n, b)| (n.as_str(), b.as_slice())).chain([("manifest.json", manifest_json.as_slice())]) {
            zip.start_file(name, options).map_err(|e| fail(&e))?;
            zip.write_all(bytes).map_err(|e| fail(&e))?;
        }
        let file = zip.finish().map_err(|e| fail(&e))?;
        file.sync_all().map_err(|e| fail(&e))
    };
    if let Err(e) = write().and_then(|_| fs::rename(&tmp, dest).map_err(|e| fail(&e))) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{DiagnosticItem, DiagnosticState};
    use std::io::Read;

    #[test]
    fn archive_is_redacted_and_matches_its_manifest() {
        let root = std::env::temp_dir().join(format!("gitpass-export-{}", uuid::Uuid::new_v4()));
        let home = root.join("Users/ana");
        let app_support = home.join("Library/Application Support/com.gitpass.GitPass");
        let logs = logs_dir(&app_support);
        fs::create_dir_all(&logs).unwrap();
        let secrets = [
            crate::test_fixtures::ghp_token_36(),
            crate::test_fixtures::github_pat_sample(),
            "Bearer test-jwt-placeholder".to_string(),
            format!("x-access-token:gho_{}@github.com", "inner"),
        ];
        let home_s = home.display().to_string();
        fs::write(
            logs.join("gitpassd.log"),
            format!(
                "INFO path={home_s}/src/app token={}\nAuthorization: {}\nurl https://{}/o/r\n-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n",
                secrets[0], secrets[2], secrets[3]
            ),
        )
        .unwrap();
        crate::audit::append_in(&app_support, crate::audit::AuditKind::EmergencyOn, None, None, &format!("motivo {}", secrets[1])).unwrap();
        crate::recent::record_in(
            &app_support,
            crate::recent::RecentEntry {
                root_path: home.join("src/app"),
                binding_id: None,
                account_id: None,
                full_name: Some("octo/app".into()),
                verdict: crate::recent::Verdict::Ok,
                code: None,
                message: None,
                at: now_rfc3339(),
            },
        )
        .unwrap();
        fs::write(logs.join(".hidden.log"), "skip").unwrap();

        let diag = Diagnostics {
            generated_at: now_rfc3339(),
            app_version: "0.1.0".into(),
            macos_version: Some("15.1".into()),
            git_version: Some("2.50.1".into()),
            items: vec![DiagnosticItem {
                id: "ssh".into(),
                label: "Chaves SSH".into(),
                state: DiagnosticState::Ok,
                detail: format!("{home_s}/.ssh ok"),
                code: None,
                action: None,
            }],
        };
        let dest = root.join("GitPass-logs-2026-11-06.zip");
        let manifest = write_logs_zip(&app_support, Some(&home), &diag, &dest).unwrap();

        let names: Vec<&str> = manifest.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["logs/audit.jsonl", "logs/gitpassd.log", "recent.json", "diagnostics.json"]);
        let mut archive = zip::ZipArchive::new(fs::File::open(&dest).unwrap()).unwrap();
        assert_eq!(archive.len(), 5);
        for f in &manifest.files {
            let mut text = String::new();
            archive.by_name(&f.name).unwrap().read_to_string(&mut text).unwrap();
            assert_eq!(sha256_hex(text.as_bytes()), f.sha256, "{}", f.name);
            assert_eq!(text.len() as u64, f.bytes);
            for s in &secrets {
                assert!(!text.contains(s), "{} leaks {s}:\n{text}", f.name);
            }
            assert!(!text.contains("gho_inner") && !text.contains("AAAA\n"), "{}:\n{text}", f.name);
            assert!(!text.contains(&home_s), "{} shows the home folder:\n{text}", f.name);
        }
        let mut log = String::new();
        archive.by_name("logs/gitpassd.log").unwrap().read_to_string(&mut log).unwrap();
        assert!(log.contains("path=~/src/app") && log.contains("[CHAVE PRIVADA REDIGIDA]"), "{log}");
        let mut m = String::new();
        archive.by_name("manifest.json").unwrap().read_to_string(&mut m).unwrap();
        let m: serde_json::Value = serde_json::from_str(&m).unwrap();
        assert_eq!(m["app_version"], "0.1.0");
        assert_eq!(m["files"].as_array().unwrap().len(), 4);
        assert!(!root.read_dir().unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains(".tmp-")));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn default_name_uses_the_local_date() {
        let d = chrono::DateTime::parse_from_rfc3339("2026-03-04T12:00:00-03:00").unwrap().with_timezone(&chrono::Local);
        assert!(default_file_name(d).starts_with("GitPass-logs-2026-03-0"));
        assert!(default_file_name(d).ends_with(".zip"));
    }
}
