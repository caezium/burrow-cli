//! Windows uninstall adapter via Bulk Crap Uninstaller (`BCU-console.exe`, Apache-2.0).
//!
//! The command-building — the verified `/Q` (quiet) `/U` (unattended) `/J=<confidence>`
//! (leftover cleanup at confidence ≥ level) flags from Phase 0 — is PURE and tested here.
//! BCU runs only on Windows, so the preview shows the exact command we'd run (cross-platform,
//! demonstrable on macOS) and `--apply` invokes BCU (fixture-tested; live on Windows).

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

pub fn resolve_bcu() -> Result<PathBuf, String> {
    if let Some(p) = platform::resolve_env_executable("BURROW_BCU", &["BCU-console"])? {
        return Ok(p);
    }
    if let Some(p) = platform::resolve_on_path(&["BCU-console"]) {
        return Ok(p);
    }
    Err("BCU-console not found (Windows; Bulk Crap Uninstaller). Set BURROW_BCU".into())
}

pub fn execute(bcu: &Path, args: &[String]) -> Result<Value, String> {
    let out = platform::command(bcu, args)?
        .output()
        .map_err(|e| format!("failed to run BCU: {e}"))?;
    Ok(json!({
        "applied": true,
        "ok": out.status.success(),
        "command": args,
        "stdout": String::from_utf8_lossy(&out.stdout).trim(),
        "stderr": String::from_utf8_lossy(&out.stderr).trim(),
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
}
