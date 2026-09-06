//! Trash sentinel (one-shot): which `.app` bundles are currently in the Trash — candidates
//! for leftover cleanup the moment they're trashed. The *continuous* FSEvents login-item
//! daemon is a packaging concern; this actionable scan is pure and tested, and is what such a
//! daemon would call on each Trash event.

use crate::output::Failure;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize, PartialEq)]
pub struct TrashedApp {
    pub name: String,
    pub path: String,
}

/// Which apps arrived since the previous tick (diff by path; removals are not announced).
/// The pure core of watch mode — what an FSEvents callback would compute per event.
pub fn new_arrivals(prev: &[TrashedApp], curr: &[TrashedApp]) -> Vec<TrashedApp> {
    curr.iter()
        .filter(|c| !prev.iter().any(|p| p.path == c.path))
        .map(|c| TrashedApp {
            name: c.name.clone(),
            path: c.path.clone(),
        })
        .collect()
}

/// List `.app` bundles directly inside a Trash directory (sorted by name).
pub fn scan_trash(dir: &Path) -> Result<Vec<TrashedApp>, Failure> {
    let mut apps = Vec::new();
    let entries = std::fs::read_dir(dir)
        .map_err(|e| Failure::io(format!("cannot read {}", dir.display()), &e))?;
    for e in entries {
        let e =
            e.map_err(|e| Failure::io(format!("cannot read an entry of {}", dir.display()), &e))?;
        let n = e.file_name().to_string_lossy().into_owned();
        if let Some(app) = n.strip_suffix(".app") {
            apps.push(TrashedApp {
                name: app.to_string(),
                path: e.path().to_string_lossy().into_owned(),
            });
        }
    }
    apps.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(apps)
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

        let apps = scan_trash(&dir).unwrap();
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].name, "Bar"); // sorted
        assert_eq!(apps[1].name, "Foo");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_arrivals_diffs_by_path() {
        let a = TrashedApp {
            name: "A".into(),
            path: "/t/A.app".into(),
        };
        let b = TrashedApp {
            name: "B".into(),
            path: "/t/B.app".into(),
        };
        let prev = vec![a];
        let curr = vec![
            TrashedApp {
                name: "A".into(),
                path: "/t/A.app".into(),
            },
            b,
        ];
        let events = new_arrivals(&prev, &curr);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "B");
        // Nothing new -> no events (removals are not announced).
        assert!(new_arrivals(&curr, &prev).is_empty());
    }

    #[test]
    fn empty_trash_yields_nothing() {
        let dir = std::env::temp_dir().join(format!("burrow_trash_empty_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(scan_trash(&dir).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
