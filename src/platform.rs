//! Cross-platform helpers for paths, executable resolution, and platform capability errors.

use crate::output::Failure;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn is_windows() -> bool {
    cfg!(windows)
}

pub fn is_macos() -> bool {
    cfg!(target_os = "macos")
}

pub fn home_dir() -> PathBuf {
    if is_windows() {
        std::env::var("USERPROFILE")
            .map(PathBuf::from)
            .or_else(|_| std::env::var("HOME").map(PathBuf::from))
            .unwrap_or_else(|_| PathBuf::from("."))
    } else {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    }
}

pub fn home_dir_string() -> String {
    home_dir().to_string_lossy().into_owned()
}

pub fn config_dir() -> PathBuf {
    if let Ok(d) = std::env::var("BURROW_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    if is_windows() {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home_dir().join("AppData").join("Roaming"))
            .join("Burrow")
    } else {
        home_dir().join(".burrow")
    }
}

fn state_base() -> PathBuf {
    if is_windows() {
        std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home_dir().join("AppData").join("Local"))
            .join("Burrow")
    } else {
        home_dir().join(".burrow")
    }
}

pub fn scan_dir() -> PathBuf {
    std::env::var("BURROW_SCAN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| state_base().join("scans"))
}

pub fn snapshot_dir() -> PathBuf {
    std::env::var("BURROW_SNAPSHOT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| state_base().join("snapshots"))
}

fn windows_exts() -> Vec<String> {
    let mut exts = vec!["exe".to_string(), "cmd".to_string(), "bat".to_string()];
    if let Ok(pathext) = std::env::var("PATHEXT") {
        for ext in pathext.split(';') {
            let trimmed = ext.trim().trim_start_matches('.').to_ascii_lowercase();
            if !trimmed.is_empty() && !exts.contains(&trimmed) {
                exts.push(trimmed);
            }
        }
    }
    exts
}

pub fn executable_path_candidates(path: &Path) -> Vec<PathBuf> {
    if is_windows() && path.extension().is_none() {
        let mut candidates: Vec<PathBuf> = windows_exts()
            .into_iter()
            .map(|ext| path.with_extension(ext))
            .collect();
        candidates.push(path.to_path_buf());
        candidates
    } else {
        vec![path.to_path_buf()]
    }
}

pub fn executable_names(name: &str) -> Vec<String> {
    let path = Path::new(name);
    if is_windows() && path.extension().is_none() {
        let mut names: Vec<String> = windows_exts()
            .into_iter()
            .map(|ext| format!("{name}.{ext}"))
            .collect();
        names.push(name.to_string());
        names
    } else {
        vec![name.to_string()]
    }
}

pub fn resolve_in_dir(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    for name in names {
        for candidate in executable_names(name) {
            let path = dir.join(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }
    None
}

pub fn resolve_on_path(names: &[&str]) -> Option<PathBuf> {
    let path = std::env::var("PATH").ok()?;
    for dir in std::env::split_paths(&path) {
        if let Some(found) = resolve_in_dir(&dir, names) {
            return Some(found);
        }
    }
    None
}

/// The executable named by `$var`: a path to the binary, or a directory that contains one of
/// `fallback_names`. `Ok(None)` when the variable is unset; a set-but-wrong override is a
/// `not_found` failure rather than a reason to search elsewhere — silently falling through to a
/// different binary than the one named is how you debug the wrong engine for an hour.
pub fn resolve_env_executable(
    var: &str,
    fallback_names: &[&str],
) -> Result<Option<PathBuf>, Failure> {
    let Ok(raw) = std::env::var(var) else {
        return Ok(None);
    };
    let path = PathBuf::from(&raw);
    if path.is_dir() {
        return resolve_in_dir(&path, fallback_names)
            .map(Some)
            .ok_or_else(|| {
                Failure::not_found(format!("${var} '{raw}' contains no usable executable"))
            });
    }
    for candidate in executable_path_candidates(&path) {
        if candidate.exists() {
            return Ok(Some(candidate));
        }
    }
    Err(Failure::not_found(format!("${var} '{raw}' does not exist")))
}

/// Spawn the resolved program directly. Rust's Windows batch-file handling
/// quotes regular arguments, disables AutoRun/delayed expansion, protects `%`
/// arguments, and refuses CR/LF. An explicit `cmd /C` bypasses those protections.
/// The PowerShell forwarding used by our batch shims cannot preserve quotes in
/// arguments, so reject those too. Batch program paths still undergo percent
/// expansion in cmd, so refuse that unsupported path shape before launch.
pub fn command<I, S>(path: &Path, arguments: I) -> Result<Command, Failure>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let program = command_path(path, is_windows())?;
    let batch = is_windows() && is_batch_program(&program);
    let mut child = Command::new(&program);
    for argument in arguments {
        let argument = argument.as_ref();
        // A quote survives cmd's first parser but can make the batch script's
        // PowerShell forwarding absorb the following --dry-run into this arg.
        // Never pass an unrepresentable argument through that second parser.
        if batch && argument.to_string_lossy().contains(['"', '\r', '\n']) {
            return Err(Failure::error("batch arguments containing quotes or newlines cannot preserve the preview guard; use a native executable"));
        }
        child.arg(argument);
    }
    Ok(child)
}

fn is_batch_program(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| matches!(extension.to_ascii_lowercase().as_str(), "cmd" | "bat"))
        .unwrap_or(false)
}

fn command_path(path: &Path, windows: bool) -> Result<PathBuf, Failure> {
    if windows && is_batch_program(path) {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| Failure::io("cannot resolve batch program path", &e))?
                .join(path)
        };
        if absolute.to_string_lossy().contains(['%', '\r', '\n', '"']) {
            return Err(Failure::error("batch program path contains unsupported shell characters; use an installation path without percent signs"));
        }
        Ok(absolute)
    } else {
        Ok(path.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_wins_for_config() {
        let dir = std::env::temp_dir().join(format!("burrow_cfg_{}", std::process::id()));
        std::env::set_var("BURROW_CONFIG_DIR", &dir);
        assert_eq!(config_dir(), dir);
        std::env::remove_var("BURROW_CONFIG_DIR");
    }

    #[test]
    fn windows_candidates_include_command_extensions_when_applicable() {
        let names = executable_names("tool");
        if is_windows() {
            assert!(names.iter().any(|n| n == "tool.exe"));
            assert!(names.iter().any(|n| n == "tool.cmd"));
            assert_eq!(names[0], "tool.exe");
            assert_eq!(names[1], "tool.cmd");
            assert_eq!(names[2], "tool.bat");
        } else {
            assert_eq!(names, vec!["tool".to_string()]);
        }
    }

    /// Keep the resolved batch program visible to Rust so its batch-specific
    /// argument escaping runs; an explicit cmd wrapper bypasses it.
    #[test]
    fn a_script_target_is_spawned_directly_with_its_argument_intact() {
        let hostile = "Foo & echo pwned";
        for name in ["burrow-engine.cmd", "burrow-engine.bat", "burrow-engine"] {
            let path = std::env::temp_dir().join(name);
            let c = command(&path, [hostile]).unwrap();
            assert_eq!(c.get_program(), path.as_os_str(), "{name}");
            let args: Vec<&std::ffi::OsStr> = c.get_args().collect();
            assert_eq!(args, vec![std::ffi::OsStr::new(hostile)], "{name}");
        }
    }
    #[test]
    fn batch_program_paths_cannot_expand_environment_variables() {
        let path = std::env::temp_dir().join("%BURROW_TEST%/engine.cmd");
        assert!(command_path(&path, true).is_err());
        assert!(command_path(&path.with_extension("bat"), true).is_err());
        assert!(command_path(&path.with_extension("exe"), true).is_ok());
        assert!(command_path(&path, false).is_ok());
    }
}
