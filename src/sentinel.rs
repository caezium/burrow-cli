//! Trash sentinel (one-shot): which `.app` bundles are currently in the Trash — candidates
//! for leftover cleanup the moment they're trashed. The *continuous* FSEvents login-item
//! daemon is a packaging concern; this actionable scan is pure and tested, and is what such a
//! daemon would call on each Trash event.

use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize, PartialEq)]
pub struct TrashedApp {
    pub name: String,
    pub path: String,
}

/// List `.app` bundles directly inside a Trash directory (sorted by name).
pub fn scan_trash(dir: &Path) -> Vec<TrashedApp> {
    let mut apps = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if let Some(app) = n.strip_suffix(".app") {
                apps.push(TrashedApp {
                    name: app.to_string(),
                    path: e.path().to_string_lossy().into_owned(),
                });
            }
        }
    }
    apps.sort_by(|a, b| a.name.cmp(&b.name));
    apps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_only_app_bundles() {
        let dir = std::env::temp_dir().join(format!("burrow_trash_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join("Foo.app")).unwrap();
        std::fs::create_dir_all(dir.join("Bar.app")).unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();

        let apps = scan_trash(&dir);
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].name, "Bar"); // sorted
        assert_eq!(apps[1].name, "Foo");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_trash_yields_nothing() {
        let dir = std::env::temp_dir().join(format!("burrow_trash_empty_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(scan_trash(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
