//! Cloud-file dehydration: evict the local copy of cloud-backed files, freeing disk while
//! keeping the cloud item available on demand. macOS wraps `brctl evict`; Windows uses the
//! OneDrive/Cloud Files `attrib +U -P` convention as a first-stage adapter.

use crate::platform;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Parse args into (paths, apply).
pub fn parse(args: &[String]) -> Result<(Vec<String>, bool), String> {
    let apply = args.iter().any(|a| a == "--apply");
    let paths: Vec<String> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect();
    if paths.is_empty() {
        return Err("evict: needs at least one path".into());
    }
    Ok((paths, apply))
}

/// Preview: report each path and whether it currently exists locally. Mutates nothing.
pub fn dry_run(paths: &[String]) -> Value {
    let items: Vec<Value> = paths
        .iter()
        .map(|p| {
            json!({
                "path": p,
                "exists": Path::new(p).exists(),
                "supported": if platform::is_windows() { cloud_hint(p) } else { platform::is_macos() },
            })
        })
        .collect();
    json!({ "applied": false, "would_evict": items })
}

fn cloud_hint(path: &str) -> bool {
    if !platform::is_windows() {
        return true;
    }
    let lower = path.to_ascii_lowercase();
    if lower.contains("onedrive") {
        return true;
    }
    for key in ["OneDrive", "OneDriveCommercial", "OneDriveConsumer"] {
        if let Ok(root) = std::env::var(key) {
            let root_lower = root.to_ascii_lowercase();
            if !root_lower.is_empty() && lower.starts_with(&root_lower) {
                return true;
            }
        }
    }
    false
}

/// Resolve the `brctl` binary: `$BURROW_BRCTL`, `/usr/bin/brctl`, else `brctl` on PATH.
pub fn resolve_brctl() -> Result<PathBuf, String> {
    if let Some(p) = platform::resolve_env_executable("BURROW_BRCTL", &["brctl"])? {
        return Ok(p);
    }
    let fixed = PathBuf::from("/usr/bin/brctl");
    if fixed.exists() {
        return Ok(fixed);
    }
    if let Some(p) = platform::resolve_on_path(&["brctl"]) {
        return Ok(p);
    }
    Err("brctl not found (macOS iCloud tool); set BURROW_BRCTL".into())
}

pub fn execute_apply(paths: &[String]) -> Result<Value, String> {
    if platform::is_windows() {
        execute_apply_windows(paths)
    } else {
        let brctl = resolve_brctl()?;
        execute_apply_brctl(&brctl, paths)
    }
}

/// Evict each path via `brctl evict <path>`. Returns a per-path result.
fn execute_apply_brctl(brctl: &Path, paths: &[String]) -> Result<Value, String> {
    let mut results = Vec::new();
    for p in paths {
        let out = platform::command(brctl, ["evict", p])?
            .output()
            .map_err(|e| format!("failed to run brctl evict {p}: {e}"))?;
        results.push(json!({
            "path": p,
            "ok": out.status.success(),
            "stderr": String::from_utf8_lossy(&out.stderr).trim(),
        }));
    }
    Ok(json!({ "applied": true, "evicted": results }))
}

fn execute_apply_windows(paths: &[String]) -> Result<Value, String> {
    let mut results = Vec::new();
    for p in paths {
        if !cloud_hint(p) {
            results.push(json!({
                "path": p,
                "ok": false,
                "unsupported": true,
                "stderr": "path is not under a detected OneDrive/Cloud Files root",
            }));
            continue;
        }
        let out = std::process::Command::new("attrib")
            .args(["+U", "-P", p])
            .output()
            .map_err(|e| format!("failed to run attrib for {p}: {e}"))?;
        results.push(json!({
            "path": p,
            "ok": out.status.success(),
            "stderr": String::from_utf8_lossy(&out.stderr).trim(),
        }));
    }
    Ok(json!({ "applied": true, "evicted": results }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_paths_and_apply() {
        let (paths, apply) = parse(&a(&["~/Doc.pdf", "--apply"])).unwrap();
        assert_eq!(paths, a(&["~/Doc.pdf"]));
        assert!(apply);
        let (_, apply2) = parse(&a(&["~/Doc.pdf"])).unwrap();
        assert!(!apply2);
    }

    #[test]
    fn parse_requires_path() {
        assert!(parse(&a(&["--apply"])).is_err());
    }

    #[test]
    fn dry_run_reports_existence_and_does_not_apply() {
        let v = dry_run(&a(&["/definitely/not/here/x.pdf"]));
        assert_eq!(v["applied"], json!(false));
        assert_eq!(v["would_evict"][0]["exists"], json!(false));
    }

    #[test]
    #[cfg(not(windows))]
    fn apply_runs_brctl_per_path() {
        let dir = std::env::temp_dir().join(format!("burrow_brctl_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join(if cfg!(windows) { "brctl.cmd" } else { "brctl" });
        let script = if cfg!(windows) {
            "@echo off\r\nexit /b 0\r\n"
        } else {
            "#!/bin/sh\nexit 0\n"
        };
        std::fs::write(&fake, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let v = execute_apply_brctl(&fake, &a(&["/some/file"])).unwrap();
        assert_eq!(v["applied"], json!(true));
        assert_eq!(v["evicted"][0]["ok"], json!(true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(windows)]
    fn windows_dry_run_marks_non_cloud_paths_unsupported() {
        let v = dry_run(&a(&["C:\\Temp\\file.txt"]));
        assert_eq!(v["applied"], json!(false));
        assert_eq!(v["would_evict"][0]["supported"], json!(false));
    }
}
