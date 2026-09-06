//! Duplicate-file engine: wraps the fclones sidecar (MIT) via its stdout->stdin round-trip.
//!
//! Verified contract (Phase 0): `fclones group --format json <paths>` emits a report to
//! stdout; `fclones dedupe|remove|link` read that report from stdin. `dedupe` uses APFS
//! `clonefile` on macOS (reclaim space without deleting). Safety: the mutating actions only
//! run with `--apply`; otherwise we return the group report as a preview.

use crate::platform;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

#[derive(Debug, PartialEq, Eq)]
pub enum DupesPlan {
    /// Read-only: `fclones group --format json <paths>`.
    Group { paths: Vec<String> },
    /// Mutating (requires --apply): `fclones group ... | fclones <action>`.
    Action {
        paths: Vec<String>,
        action: &'static str,
    },
}

fn is_flag(a: &str) -> bool {
    a.starts_with("--")
}

/// Map a `dupes` subcommand + args to a plan. Without `--apply`, the mutating subcommands
/// (dedupe/remove/link) degrade to a read-only `Group` preview.
pub fn plan(sub: &str, args: &[String]) -> Result<DupesPlan, String> {
    let apply = args.iter().any(|a| a == "--apply");
    let paths: Vec<String> = args.iter().filter(|a| !is_flag(a)).cloned().collect();
    if paths.is_empty() {
        return Err("dupes: needs at least one path to scan".into());
    }
    match sub {
        "group" => Ok(DupesPlan::Group { paths }),
        "dedupe" | "remove" | "link" => {
            if apply {
                let action = match sub {
                    "dedupe" => "dedupe",
                    "remove" => "remove",
                    _ => "link",
                };
                Ok(DupesPlan::Action { paths, action })
            } else {
                Ok(DupesPlan::Group { paths }) // preview, do not mutate
            }
        }
        other => Err(format!("dupes: unknown subcommand '{other}'")),
    }
}

/// Resolve the fclones binary: `$BURROW_FCLONES`, else `fclones` on `PATH`.
pub fn resolve_fclones() -> Result<PathBuf, String> {
    if let Some(p) = platform::resolve_env_executable("BURROW_FCLONES", &["fclones"])? {
        return Ok(p);
    }
    if let Some(p) = platform::resolve_on_path(&["fclones"]) {
        return Ok(p);
    }
    Err("fclones not found; install it (cargo install fclones) or set BURROW_FCLONES".into())
}

fn group_report(fclones: &Path, paths: &[String]) -> Result<String, String> {
    let mut args = vec![
        "group".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];
    args.extend(paths.iter().cloned());
    let out = platform::command(fclones, &args)?
        .output()
        .map_err(|e| format!("failed to run fclones group: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "fclones group exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Execute a plan and return the resulting JSON (group report, or action result).
pub fn execute(fclones: &Path, plan: &DupesPlan) -> Result<String, String> {
    match plan {
        DupesPlan::Group { paths } => group_report(fclones, paths),
        DupesPlan::Action { paths, action } => {
            let report = group_report(fclones, paths)?;
            let mut child = platform::command(fclones, [action])?
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("failed to spawn fclones {action}: {e}"))?;
            child
                .stdin
                .take()
                .expect("piped stdin")
                .write_all(report.as_bytes())
                .map_err(|e| format!("failed to stream report to fclones {action}: {e}"))?;
            let out = child
                .wait_with_output()
                .map_err(|e| format!("fclones {action} failed: {e}"))?;
            if !out.status.success() {
                return Err(format!(
                    "fclones {action} exited {}: {}",
                    out.status,
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn group_is_default_readonly() {
        assert_eq!(
            plan("group", &a(&["/tmp"])).unwrap(),
            DupesPlan::Group {
                paths: a(&["/tmp"])
            }
        );
    }

    #[test]
    fn dedupe_without_apply_is_preview() {
        assert_eq!(
            plan("dedupe", &a(&["/tmp"])).unwrap(),
            DupesPlan::Group {
                paths: a(&["/tmp"])
            }
        );
    }

    #[test]
    fn dedupe_apply_is_action() {
        assert_eq!(
            plan("dedupe", &a(&["/tmp", "--apply"])).unwrap(),
            DupesPlan::Action {
                paths: a(&["/tmp"]),
                action: "dedupe"
            }
        );
    }

    #[test]
    fn remove_and_link_apply() {
        assert_eq!(
            plan("remove", &a(&["--apply", "/tmp"])).unwrap(),
            DupesPlan::Action {
                paths: a(&["/tmp"]),
                action: "remove"
            }
        );
        assert_eq!(
            plan("link", &a(&["/tmp", "--apply"])).unwrap(),
            DupesPlan::Action {
                paths: a(&["/tmp"]),
                action: "link"
            }
        );
    }

    #[test]
    fn needs_a_path() {
        assert!(plan("group", &[]).is_err());
        assert!(plan("dedupe", &a(&["--apply"])).is_err());
    }

    #[test]
    fn unknown_subcommand_errors() {
        assert!(plan("frobnicate", &a(&["/tmp"])).is_err());
    }
}
