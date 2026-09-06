//! Launch (or install) the Burrow GUI app.
//!
//! Locates `Burrow.app` (fixed locations, then Spotlight by bundle id) and opens it. With
//! `--install`, installs the Homebrew cask first. Pure-ish: `locate()` is testable; launch
//! and install shell out to `open` / `brew`.

use std::path::{Path, PathBuf};
use std::process::Command;

pub const BUNDLE_ID: &str = "dev.caezium.Burrow";
pub const CASK: &str = "caezium/tap/burrow";
pub const SITE: &str = "https://burrow.computer";

/// Locate `Burrow.app`: /Applications, ~/Applications, then `mdfind` by bundle id.
pub fn locate() -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from("/Applications/Burrow.app")];
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(PathBuf::from(home).join("Applications/Burrow.app"));
    }
    for c in &candidates {
        if c.is_dir() {
            return Some(c.clone());
        }
    }
    // Spotlight: find it wherever it lives.
    if let Ok(out) = Command::new("mdfind")
        .arg(format!("kMDItemCFBundleIdentifier == '{BUNDLE_ID}'"))
        .output()
    {
        if let Some(first) = String::from_utf8_lossy(&out.stdout).lines().next() {
            let p = PathBuf::from(first.trim());
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    None
}

/// Open the app via `open`.
pub fn launch(path: &Path) -> Result<(), String> {
    let st = Command::new("open")
        .arg(path)
        .status()
        .map_err(|e| format!("failed to run `open`: {e}"))?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("`open` exited {st}"))
    }
}

/// Install the Homebrew cask (interactive — brew output streams to the terminal).
pub fn install() -> Result<(), String> {
    eprintln!("burrow gui: installing via Homebrew — brew install --cask {CASK}");
    let st = Command::new("brew")
        .args(["install", "--cask", CASK])
        .status()
        .map_err(|e| format!("failed to run `brew` (is Homebrew installed?): {e}"))?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("`brew install --cask {CASK}` exited {st}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_are_sane() {
        assert!(BUNDLE_ID.starts_with("dev.caezium."));
        assert!(CASK.contains("/")); // tap-qualified
    }

    #[test]
    fn locate_returns_dir_or_none() {
        // Never panics; returns an existing .app dir or None.
        if let Some(p) = locate() {
            assert!(p.is_dir());
            assert!(p.extension().map(|e| e == "app").unwrap_or(false));
        }
    }
}
