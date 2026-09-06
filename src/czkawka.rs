//! Windows duplicate / similar-photo adapter via `czkawka_cli` (MIT core).
//!
//! On Windows, fclones can't reflink-dedupe (no clonefile), so czkawka_cli is the duplicate +
//! similar-image engine. Command-building (the verified flags from Phase 0: `dup -C` compact
//! JSON; `image --pretty-file-to-save`) is pure + tested; czkawka writes JSON to a file we read.
//! czkawka_cli is itself cross-platform, so this resolver/plan also works on macOS if installed.

use crate::platform;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub enum Scan {
    Duplicates,
    SimilarImages,
}

/// Build the `czkawka_cli` argument vector writing results to `out_json`.
pub fn plan(scan: &Scan, dir: &str, out_json: &str) -> Result<Vec<String>, String> {
    if dir.trim().is_empty() {
        return Err("win-dupes: needs a directory".into());
    }
    Ok(match scan {
        Scan::Duplicates => vec![
            "dup".into(),
            "-d".into(),
            dir.into(),
            "-C".into(),
            out_json.into(), // compact JSON
        ],
        Scan::SimilarImages => vec![
            "image".into(),
            "-d".into(),
            dir.into(),
            "-c".into(),
            "16".into(), // perceptual hash size
            "--pretty-file-to-save".into(),
            out_json.into(),
        ],
    })
}

pub fn resolve_czkawka() -> Result<PathBuf, String> {
    if let Some(p) = platform::resolve_env_executable("BURROW_CZKAWKA", &["czkawka_cli"])? {
        return Ok(p);
    }
    if let Some(p) = platform::resolve_on_path(&["czkawka_cli"]) {
        return Ok(p);
    }
    Err("czkawka_cli not found; install it or set BURROW_CZKAWKA".into())
}

/// Run czkawka writing to `out_json`, then read+return that JSON (czkawka writes to a file,
/// not stdout — unlike fclones).
pub fn execute(czkawka: &Path, args: &[String], out_json: &str) -> Result<Value, String> {
    let out = platform::command(czkawka, args)?
        .output()
        .map_err(|e| format!("failed to run czkawka_cli: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "czkawka exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let report = std::fs::read_to_string(out_json)
        .map_err(|e| format!("czkawka wrote no readable report at {out_json}: {e}"))?;
    Ok(
        json!({ "ok": true, "report": serde_json::from_str::<Value>(&report).unwrap_or(json!(report)) }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn duplicates_uses_compact_json() {
        assert_eq!(
            plan(&Scan::Duplicates, "C:/Users/x", "out.json").unwrap(),
            v(&["dup", "-d", "C:/Users/x", "-C", "out.json"])
        );
    }

    #[test]
    fn similar_images_uses_pretty_save_and_hash_size() {
        assert_eq!(
            plan(&Scan::SimilarImages, "C:/Pics", "sim.json").unwrap(),
            v(&[
                "image",
                "-d",
                "C:/Pics",
                "-c",
                "16",
                "--pretty-file-to-save",
                "sim.json"
            ])
        );
    }

    #[test]
    fn needs_a_directory() {
        assert!(plan(&Scan::Duplicates, "  ", "o.json").is_err());
    }
}
