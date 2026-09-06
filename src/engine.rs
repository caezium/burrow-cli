//! Engine resolution, command planning, and invocation.
//!
//! burrow-engine is invoked as a separate process (arm's-length): nothing from it is linked
//! into the conductor, which keeps the MIT engine and the FSL conductor cleanly separated.
//! `plan()` is the pure, unit-tested core that maps a conductor command to an engine call.

use crate::platform;
use std::path::{Path, PathBuf};

/// Where a planned command runs inside the engine.
#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    /// The bash entrypoint `mole` (clean/uninstall/optimize/purge/installer/history).
    Bash,
    /// A bundled Go binary under `bin/` (e.g. "status-go", "analyze-go").
    GoBinary(&'static str),
}

/// A fully-resolved plan for one conductor command: which engine target + exact args.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub target: Target,
    pub args: Vec<String>,
}

/// Conductor-level flags that never reach the engine.
fn is_conductor_flag(a: &str) -> bool {
    matches!(a, "--apply" | "--json" | "--raw")
}

/// Map a burrow command + user args to an engine invocation.
///
/// Safety model (explain-before-delete): destructive commands
/// (clean/purge/optimize/uninstall/installer) default to the engine's `--dry-run`; passing
/// `--apply` removes the guard. Read-only commands (status/analyze/history) always run.
pub fn plan(command: &str, user_args: &[String]) -> Result<Plan, String> {
    let apply = user_args.iter().any(|a| a == "--apply");
    let passthrough: Vec<String> = user_args
        .iter()
        .filter(|a| !is_conductor_flag(a))
        .cloned()
        .collect();

    match command {
        "status" => Ok(Plan {
            target: Target::GoBinary("status-go"),
            args: vec!["--json".into()],
        }),
        "analyze" => {
            let mut args = vec!["--json".to_string()];
            args.extend(passthrough); // optional positional <path>
            Ok(Plan {
                target: Target::GoBinary("analyze-go"),
                args,
            })
        }
        "clean" => Ok(dry_or_apply("clean", apply, &[])),
        "optimize" => Ok(dry_or_apply("optimize", apply, &[])),
        "installer" => Ok(dry_or_apply("installer", apply, &[])),
        "purge" => Ok(dry_or_apply("purge", apply, &passthrough)),
        "uninstall" => {
            if passthrough.is_empty() {
                return Err("uninstall: needs at least one app name".into());
            }
            let mut args = vec!["uninstall".to_string()];
            args.extend(passthrough);
            if !apply {
                args.push("--dry-run".into());
            }
            Ok(Plan {
                target: Target::Bash,
                args,
            })
        }
        "history" => {
            let mut args = vec!["history".to_string(), "--json".to_string()];
            args.extend(passthrough); // e.g. --limit N
            Ok(Plan {
                target: Target::Bash,
                args,
            })
        }
        other => Err(format!("unknown command '{other}'")),
    }
}

fn dry_or_apply(sub: &str, apply: bool, extra: &[String]) -> Plan {
    let mut args = vec![sub.to_string()];
    args.extend(extra.iter().cloned());
    if !apply {
        args.push("--dry-run".into());
    }
    Plan {
        target: Target::Bash,
        args,
    }
}

fn bash_entrypoint_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["burrow-engine", "mole"]
    } else {
        &["mole", "burrow-engine"]
    }
}

/// Resolve the engine directory: `$BURROW_ENGINE_DIR`, else a sibling `../burrow-engine`
/// discovered by walking up from the executable.
pub fn resolve_dir() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("BURROW_ENGINE_DIR") {
        let pb = PathBuf::from(&p);
        return if pb.is_dir() {
            Ok(pb)
        } else {
            Err(format!("$BURROW_ENGINE_DIR '{p}' is not a directory"))
        };
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(Path::to_path_buf);
        while let Some(d) = dir {
            let cand = d.join("burrow-engine");
            if platform::resolve_in_dir(&cand, bash_entrypoint_names()).is_some()
                || platform::resolve_in_dir(&cand.join("bin"), &["status-go", "analyze-go"])
                    .is_some()
            {
                return Ok(cand);
            }
            dir = d.parent().map(Path::to_path_buf);
        }
    }
    Err("could not locate burrow-engine; set BURROW_ENGINE_DIR=/path/to/burrow-engine".into())
}

/// Resolve the concrete executable + args for a plan within an engine dir.
fn resolve_exec(dir: &Path, plan: &Plan) -> Result<(PathBuf, Vec<String>), String> {
    let bin = match plan.target {
        Target::Bash => platform::resolve_in_dir(dir, bash_entrypoint_names()),
        Target::GoBinary(name) => platform::resolve_in_dir(&dir.join("bin"), &[name]),
    };
    bin.map(|b| (b, plan.args.clone())).ok_or_else(|| {
        format!(
            "engine executable not found in {} (set BURROW_ENGINE_DIR or build the Windows/macOS engine binaries)",
            dir.display()
        )
    })
}

/// Run a plan with the terminal inherited - the engine renders its native TUI / colored
/// output and reads input directly. Returns the engine's exit code.
pub fn execute_native(dir: &Path, plan: &Plan) -> Result<i32, String> {
    let (bin, args) = resolve_exec(dir, plan)?;
    let status = platform::command(&bin, &args)?
        .status()
        .map_err(|e| format!("failed to run {}: {e}", bin.display()))?;
    Ok(status.code().unwrap_or(1))
}

/// Execute a plan, returning the engine's stdout.
pub fn execute(dir: &Path, plan: &Plan) -> Result<String, String> {
    let (bin, args) = resolve_exec(dir, plan)?;
    let out = platform::command(&bin, &args)?
        .output()
        .map_err(|e| format!("failed to run {}: {e}", bin.display()))?;
    decode_output(&bin, out)
}

/// Scheduled status collection must finish even when the resolved engine stalls.
/// Keep the directory adapter and its platform-specific argv handling intact;
/// interactive and watch calls retain their native execution path.
pub fn execute_with_timeout(
    dir: &Path,
    plan: &Plan,
    timeout: std::time::Duration,
) -> Result<String, String> {
    let (bin, args) = resolve_exec(dir, plan)?;
    let out = platform::output_with_timeout(platform::command(&bin, &args)?, timeout)?;
    decode_output(&bin, out)
}

fn decode_output(bin: &Path, out: std::process::Output) -> Result<String, String> {
    if !out.status.success() {
        return Err(format!(
            "engine {} exited {}: {}",
            bin.display(),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(all(test, windows))]
fn touch(path: &Path) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, "").unwrap();
}

#[cfg(all(test, windows))]
fn temp_engine_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("burrow_engine_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;
    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[cfg(unix)]
    #[test]
    fn bounded_status_preserves_the_selected_engine_directory_and_error_contract() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "burrow selected engine {} {id}",
            std::process::id()
        ));
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let bin = dir.join("bin/status-go");
        std::fs::write(&bin, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        let plan = plan("status", &[]).unwrap();
        assert_eq!(
            execute_with_timeout(&dir, &plan, Duration::from_secs(2)).unwrap(),
            "--json\n"
        );

        std::fs::write(&bin, "#!/bin/sh\nprintf 'fixture failure' >&2\nexit 7\n").unwrap();
        let error = execute_with_timeout(&dir, &plan, Duration::from_secs(2)).unwrap_err();
        assert!(
            error.contains("exited") && error.contains("fixture failure"),
            "{error}"
        );

        std::fs::write(&bin, "#!/bin/sh\nexec /bin/sleep 10\n").unwrap();
        let started = Instant::now();
        let error = execute_with_timeout(&dir, &plan, Duration::from_millis(100)).unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn bounded_status_preserves_batch_resolution_and_arguments() {
        let dir = temp_engine_dir("bounded_status_batch");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(
            dir.join("bin/status-go.cmd"),
            "@echo off\r\necho %*\r\nexit /b 0\r\n",
        )
        .unwrap();
        let output = execute_with_timeout(
            &dir,
            &plan("status", &[]).unwrap(),
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(output.trim(), "--json");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn status_maps_to_go_json() {
        let p = plan("status", &[]).unwrap();
        assert_eq!(p.target, Target::GoBinary("status-go"));
        assert_eq!(p.args, a(&["--json"]));
    }

    #[test]
    fn analyze_passes_positional_path() {
        let p = plan("analyze", &a(&["/tmp"])).unwrap();
        assert_eq!(p.target, Target::GoBinary("analyze-go"));
        assert_eq!(p.args, a(&["--json", "/tmp"]));
    }

    #[test]
    fn clean_defaults_to_dry_run() {
        let p = plan("clean", &[]).unwrap();
        assert_eq!(p.target, Target::Bash);
        assert_eq!(p.args, a(&["clean", "--dry-run"]));
    }

    #[test]
    fn apply_removes_dry_run_guard() {
        assert_eq!(plan("clean", &a(&["--apply"])).unwrap().args, a(&["clean"]));
    }

    #[test]
    fn optimize_uses_dry_run_not_n() {
        assert_eq!(
            plan("optimize", &[]).unwrap().args,
            a(&["optimize", "--dry-run"])
        );
    }

    #[test]
    fn purge_forwards_extra_flags() {
        let p = plan("purge", &a(&["--include-empty"])).unwrap();
        assert_eq!(p.args, a(&["purge", "--include-empty", "--dry-run"]));
    }

    #[test]
    fn uninstall_requires_app_and_defaults_dry_run() {
        assert!(plan("uninstall", &[]).is_err());
        let p = plan("uninstall", &a(&["Foo.app"])).unwrap();
        assert_eq!(p.args, a(&["uninstall", "Foo.app", "--dry-run"]));
    }

    #[test]
    fn history_is_json_with_passthrough() {
        let p = plan("history", &a(&["--limit", "5"])).unwrap();
        assert_eq!(p.target, Target::Bash);
        assert_eq!(p.args, a(&["history", "--json", "--limit", "5"]));
    }

    #[test]
    fn unknown_command_errors() {
        assert!(plan("bogus", &[]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_bash_prefers_burrow_engine_before_legacy_mole() {
        let dir = temp_engine_dir("prefers_burrow_engine");
        touch(&dir.join("mole.exe"));
        touch(&dir.join("burrow-engine.cmd"));
        let plan = Plan {
            target: Target::Bash,
            args: Vec::new(),
        };
        let (bin, _) = resolve_exec(&dir, &plan).unwrap();
        assert_eq!(bin.file_name().unwrap(), "burrow-engine.cmd");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_bash_orders_exe_cmd_bat_then_extensionless() {
        let dir = temp_engine_dir("orders_extensions");
        touch(&dir.join("burrow-engine.bat"));
        touch(&dir.join("burrow-engine.cmd"));
        touch(&dir.join("burrow-engine.exe"));
        let plan = Plan {
            target: Target::Bash,
            args: Vec::new(),
        };
        let (bin, _) = resolve_exec(&dir, &plan).unwrap();
        assert_eq!(bin.file_name().unwrap(), "burrow-engine.exe");

        std::fs::remove_file(dir.join("burrow-engine.exe")).unwrap();
        std::fs::remove_file(dir.join("burrow-engine.cmd")).unwrap();
        std::fs::remove_file(dir.join("burrow-engine.bat")).unwrap();
        touch(&dir.join("burrow-engine"));
        let (bin, _) = resolve_exec(&dir, &plan).unwrap();
        assert_eq!(bin.file_name().unwrap(), "burrow-engine");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_go_binary_uses_windows_extensions() {
        let dir = temp_engine_dir("go_extensions");
        touch(&dir.join("bin").join("status-go.cmd"));
        touch(&dir.join("bin").join("status-go"));
        let plan = Plan {
            target: Target::GoBinary("status-go"),
            args: Vec::new(),
        };
        let (bin, _) = resolve_exec(&dir, &plan).unwrap();
        assert_eq!(bin.file_name().unwrap(), "status-go.cmd");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
