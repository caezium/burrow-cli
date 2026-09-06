//! Windows uninstall adapter via Bulk Crap Uninstaller (`BCU-console.exe`, Apache-2.0).
//!
//! The command-building — the verified `/Q` (quiet) `/U` (unattended) `/J=<confidence>`
//! (leftover cleanup at confidence ≥ level) flags from Phase 0 — is PURE and tested here.
//! BCU runs only on Windows, so the preview shows the exact command we'd run (cross-platform,
//! demonstrable on macOS) and `--apply` invokes BCU (fixture-tested; live on Windows).

use crate::output::{ErrorKind, Failure};
use crate::platform;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const CONFIDENCE: [&str; 5] = ["VeryGood", "Good", "Questionable", "Bad", "Unknown"];

/// Build the `BCU-console.exe` argument vector. Without apply → a read-only `list` query
/// (preview); with apply → an unattended `uninstall` + leftover cleanup at `confidence`.
pub fn plan(app: &str, confidence: &str, apply: bool) -> Result<Vec<String>, String> {
    if app.trim().is_empty() {
        return Err("win-uninstall: needs an app name".into());
    }
    if !CONFIDENCE.contains(&confidence) {
        return Err(format!(
            "invalid confidence '{confidence}' (one of {})",
            CONFIDENCE.join("|")
        ));
    }
    if apply {
        Ok(vec![
            "uninstall".into(),
            app.into(),
            "/Q".into(),
            "/U".into(),
            format!("/J={confidence}"),
        ])
    } else {
        Ok(vec!["list".into(), app.into()])
    }
}

pub fn resolve_bcu() -> Result<PathBuf, Failure> {
    if let Some(p) = platform::resolve_env_executable("BURROW_BCU", &["BCU-console"])? {
        return Ok(p);
    }
    if let Some(p) = platform::resolve_on_path(&["BCU-console"]) {
        return Ok(p);
    }
    Err(Failure::not_found(
        "BCU-console not found (Windows; Bulk Crap Uninstaller). Set BURROW_BCU",
    ))
}

/// The `error.details` a failed (or never-started) BCU apply carries: the program and argv that
/// were (to be) run, plus `engine` and `applied` so the details stand on their own. Narrowly
/// scoped on purpose — an agent can audit the attempted operation without receiving unrelated
/// environment data.
pub fn execution_details(
    bcu: &Path,
    args: &[String],
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> Value {
    json!({
        "engine": "bcu",
        "applied": true,
        "program": bcu.to_string_lossy(),
        "args": args,
        "exit_code": exit_code,
        "stdout": stdout,
        "stderr": stderr,
    })
}

/// What a non-zero BCU exit was.
///
/// BCU has no machine-readable failure channel, so this is the one place the conductor still
/// reads a process's TEXT to classify it — and it reads BCU's, not its own: the Win32
/// `ERROR_ACCESS_DENIED` (5) and `ERROR_ELEVATION_REQUIRED` (740) codes, and the phrases Windows
/// and BCU print for a denied or UAC-gated operation. Everything else is a plain process failure.
fn exit_kind(exit_code: Option<i32>, diagnostic: &str) -> ErrorKind {
    let lower = diagnostic.to_ascii_lowercase();
    if matches!(exit_code, Some(5) | Some(740))
        || lower.contains("access is denied")
        || lower.contains("permission denied")
        || lower.contains("requires elevation")
        || lower.contains("elevation")
        || lower.contains("uac")
    {
        ErrorKind::PermissionDenied
    } else {
        ErrorKind::ProcessFailed
    }
}

pub fn execute(bcu: &Path, args: &[String]) -> Result<Value, Failure> {
    let out = platform::command(bcu, args)
        .and_then(|mut command| {
            command
                .output()
                .map_err(|e| Failure::io("failed to run BCU", &e))
        })
        .map_err(|e| e.with_details(execution_details(bcu, args, None, "", "")))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let details = execution_details(bcu, args, out.status.code(), &stdout, &stderr);

    if !out.status.success() {
        let diagnostic = if stderr.is_empty() { &stdout } else { &stderr };
        return Err(Failure::new(
            exit_kind(out.status.code(), diagnostic),
            format!("BCU exited {}: {diagnostic}", out.status),
        )
        .with_details(details));
    }

    // `ok` and `engine` are the ENVELOPE's — `data` does not restate them.
    Ok(json!({
        "available": true,
        "applied": true,
        "program": bcu.to_string_lossy(),
        "args": args,
        "exit_code": out.status.code(),
        "stdout": stdout,
        "stderr": stderr,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn preview_is_readonly_list() {
        assert_eq!(
            plan("Foo App", "Good", false).unwrap(),
            v(&["list", "Foo App"])
        );
    }

    #[test]
    fn apply_is_unattended_uninstall_with_leftovers() {
        assert_eq!(
            plan("Foo App", "Good", true).unwrap(),
            v(&["uninstall", "Foo App", "/Q", "/U", "/J=Good"])
        );
    }

    #[test]
    fn confidence_levels_are_validated() {
        assert!(plan("Foo", "Good", true).is_ok());
        assert!(plan("Foo", "VeryGood", true).is_ok());
        assert!(plan("Foo", "Sloppy", true).is_err()); // not a BCU tier
    }

    #[test]
    fn empty_app_errors() {
        assert!(plan("  ", "Good", true).is_err());
    }

    /// Access denial is read off BCU's own exit code and text; anything else BCU says is a
    /// generic process failure.
    #[test]
    fn a_denied_exit_is_permission_denied_and_the_rest_is_process_failed() {
        assert_eq!(exit_kind(Some(5), ""), ErrorKind::PermissionDenied);
        assert_eq!(exit_kind(Some(740), ""), ErrorKind::PermissionDenied);
        assert_eq!(
            exit_kind(Some(1), "The requested operation requires elevation"),
            ErrorKind::PermissionDenied
        );
        assert_eq!(
            exit_kind(Some(7), "Access is denied"),
            ErrorKind::PermissionDenied
        );
        assert_eq!(
            exit_kind(Some(7), "installer crashed"),
            ErrorKind::ProcessFailed
        );
        assert_eq!(exit_kind(None, ""), ErrorKind::ProcessFailed);
    }
}
