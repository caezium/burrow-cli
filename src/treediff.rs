//! Snapshot tree-diff: "what grew since last scan".
//!
//! `diff()` is pure and tested. `scan_tree()` walks a directory accumulating per-directory
//! allocated size (block-based, depth-capped). Scans are persisted as JSON (no SQLite dep)
//! under `$BURROW_SCAN_DIR` (default `~/.burrow/scans`), so a re-scan is a diff, not a rescan.

use crate::platform;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DirSize {
    pub path: String,
    pub size: i64,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Delta {
    pub path: String,
    pub prev: i64,
    pub curr: i64,
    pub delta: i64,
    pub status: &'static str, // "grew" | "shrank" | "new" | "deleted"
}

/// Diff two scans: per-path size deltas, nonzero only, sorted by magnitude desc.
pub fn diff(prev: &[DirSize], curr: &[DirSize]) -> Vec<Delta> {
    use std::collections::HashMap;
    let pmap: HashMap<&str, i64> = prev.iter().map(|d| (d.path.as_str(), d.size)).collect();
    let cmap: HashMap<&str, i64> = curr.iter().map(|d| (d.path.as_str(), d.size)).collect();

    let mut paths: Vec<&str> = pmap.keys().chain(cmap.keys()).copied().collect();
    paths.sort();
    paths.dedup();

    let mut out = Vec::new();
    for p in paths {
        let (prev_s, curr_s, status) = match (pmap.get(p).copied(), cmap.get(p).copied()) {
            (Some(a), Some(b)) if a == b => continue,
            (Some(a), Some(b)) => (a, b, if b > a { "grew" } else { "shrank" }),
            (None, Some(b)) => (0, b, "new"),
            (Some(a), None) => (a, 0, "deleted"),
            (None, None) => continue,
        };
        out.push(Delta {
            path: p.to_string(),
            prev: prev_s,
            curr: curr_s,
            delta: curr_s - prev_s,
            status,
        });
    }
    out.sort_by_key(|delta| Reverse(delta.delta.abs()));
    out
}

#[cfg(unix)]
fn alloc_size(meta: &std::fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    meta.blocks() as i64 * 512
}
#[cfg(not(unix))]
fn alloc_size(meta: &std::fs::Metadata) -> i64 {
    meta.len() as i64
}

/// Walk `root`, emitting a `DirSize` per directory (allocated bytes of its subtree), capped at
/// `max_depth`. Symlinks are skipped.
pub fn scan_tree(root: &Path, max_depth: usize) -> Vec<DirSize> {
    let mut out = Vec::new();
    accumulate(root, 0, max_depth, &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn accumulate(dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<DirSize>) -> i64 {
    let mut total = 0i64;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    for e in entries.flatten() {
        let Ok(meta) = std::fs::symlink_metadata(e.path()) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            total += accumulate(&e.path(), depth + 1, max_depth, out);
        } else {
            total += alloc_size(&meta);
        }
    }
    if depth <= max_depth {
        out.push(DirSize {
            path: dir.to_string_lossy().into_owned(),
            size: total,
        });
    }
    total
}

#[derive(Serialize, Deserialize)]
struct ScanFile {
    root: String,
    ts: i64,
    dirs: Vec<DirSize>,
}

fn scan_dir() -> PathBuf {
    platform::scan_dir()
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn sanitize(root: &str) -> String {
    root.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Persist a scan; returns the timestamp used.
pub fn save_scan(root: &str, dirs: &[DirSize]) -> Result<i64, String> {
    let ts = now_secs();
    let dir = scan_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("{}-{}.json", sanitize(root), ts));
    let sf = ScanFile {
        root: root.to_string(),
        ts,
        dirs: dirs.to_vec(),
    };
    std::fs::write(
        &file,
        serde_json::to_string(&sf).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(ts)
}

/// Load the most recent saved scan for `root`, if any.
pub fn latest_scan(root: &str) -> Option<(i64, Vec<DirSize>)> {
    let entries = std::fs::read_dir(scan_dir()).ok()?;
    let mut best: Option<ScanFile> = None;
    for e in entries.flatten() {
        if e.path().extension().map(|x| x == "json").unwrap_or(false) {
            if let Ok(s) = std::fs::read_to_string(e.path()) {
                if let Ok(sf) = serde_json::from_str::<ScanFile>(&s) {
                    if sf.root == root && best.as_ref().map(|b| sf.ts > b.ts).unwrap_or(true) {
                        best = Some(sf);
                    }
                }
            }
        }
    }
    best.map(|sf| (sf.ts, sf.dirs))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn d(path: &str, size: i64) -> DirSize {
        DirSize {
            path: path.into(),
            size,
        }
    }

    #[test]
    fn diff_detects_grow_new_delete_sorted() {
        let prev = vec![d("/a", 100), d("/b", 50)];
        let curr = vec![d("/a", 300), d("/c", 20)];
        let out = diff(&prev, &curr);
        // /a grew +200 (largest), /b deleted -50, /c new +20
        assert_eq!(out[0].path, "/a");
        assert_eq!(out[0].delta, 200);
        assert_eq!(out[0].status, "grew");
        let by_path: std::collections::HashMap<_, _> =
            out.iter().map(|x| (x.path.as_str(), x)).collect();
        assert_eq!(by_path["/b"].status, "deleted");
        assert_eq!(by_path["/c"].status, "new");
        // sorted by |delta| desc
        assert!(out[0].delta.abs() >= out[1].delta.abs());
    }

    #[test]
    fn diff_ignores_unchanged() {
        let same = vec![d("/a", 100)];
        assert!(diff(&same, &same).is_empty());
    }

    #[test]
    fn scan_tree_counts_files() {
        let dir = std::env::temp_dir().join(format!("burrow_tt_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("f.bin"), vec![0u8; 10_000]).unwrap();
        let scan = scan_tree(&dir, 2);
        let root = scan
            .iter()
            .find(|x| x.path == dir.to_string_lossy())
            .unwrap();
        assert!(
            root.size >= 10_000,
            "root size {} should cover the file",
            root.size
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_then_latest_roundtrips() {
        let scans = std::env::temp_dir().join(format!("burrow_ss_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scans);
        std::env::set_var("BURROW_SCAN_DIR", &scans);
        let dirs = vec![d("/x", 42)];
        save_scan("/some/root", &dirs).unwrap();
        let (_, loaded) = latest_scan("/some/root").unwrap();
        assert_eq!(loaded, dirs);
        assert!(latest_scan("/other/root").is_none());
        std::env::remove_var("BURROW_SCAN_DIR");
        let _ = std::fs::remove_dir_all(&scans);
    }
}
