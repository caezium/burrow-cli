//! Cross-platform safe-delete adapter: moves files to the **Recycle Bin** on Windows and the
//! **Trash** on macOS/Linux, via the `trash` crate. This is the Phase 7 "Recycle Bin" adapter
//! implemented as ONE cross-platform code path — testable on macOS, and it compiles for Windows
//! (`cargo build --target x86_64-pc-windows-gnu`), where the same call uses the Recycle Bin.
//! Reversible by design (unlike a hard delete). Default preview; `--apply` performs the move.

use serde_json::{json, Value};
use std::path::Path;

pub fn parse(args: &[String]) -> Result<(Vec<String>, bool), String> {
    let apply = args.iter().any(|a| a == "--apply");
    let paths: Vec<String> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect();
    if paths.is_empty() {
        return Err("trash: needs at least one path".into());
    }
    Ok((paths, apply))
}

/// The platform destination name (for the envelope).
pub fn destination() -> &'static str {
    if cfg!(target_os = "windows") {
        "Recycle Bin"
    } else {
        "Trash"
    }
}

/// Preview: report each path + whether it exists. Mutates nothing.
pub fn dry_run(paths: &[String]) -> Value {
    let items: Vec<Value> = paths
        .iter()
        .map(|p| json!({ "path": p, "exists": Path::new(p).exists() }))
        .collect();
    json!({ "applied": false, "destination": destination(), "would_trash": items })
}

/// A per-path safe-delete, injectable so plan execution can be tested without
/// touching the real Trash / Recycle Bin.
pub trait Delete {
    fn delete(&self, path: &str) -> Result<(), String>;
}

/// Production deleter: the `trash` crate (Recycle Bin on Windows, Trash elsewhere).
pub struct SystemTrash;
impl Delete for SystemTrash {
    fn delete(&self, path: &str) -> Result<(), String> {
        trash::delete(path).map_err(|e| e.to_string())
    }
}

/// Trash each path in order, invoking `on_item` with the per-path result JSON as
/// each completes — so a caller can STREAM live progress instead of waiting for the
/// batch. Each event matches `trash_all`'s `trashed[]` entries.
pub fn trash_each(paths: &[String], deleter: &dyn Delete, mut on_item: impl FnMut(Value)) {
    for p in paths {
        let r = deleter.delete(p);
        on_item(json!({ "path": p, "ok": r.is_ok(), "error": r.err() }));
    }
}

/// Move each path to the Recycle Bin / Trash; collected per-path result.
pub fn trash_all(paths: &[String]) -> Value {
    let mut results = Vec::new();
    trash_each(paths, &SystemTrash, |e| results.push(e));
    json!({ "applied": true, "destination": destination(), "trashed": results })
}

/// Extract the deletion paths from a held `trash` plan, so a caller re-applies the
/// exact set without re-scanning ("scan once, execute the plan"). Accepts a prior
/// preview `{would_trash:[{path},…]}` or a bare JSON array of path strings.
pub fn plan_paths(json: &str) -> Result<Vec<String>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("bad plan JSON: {e}"))?;
    if let Some(items) = v.get("would_trash").and_then(Value::as_array) {
        Ok(items
            .iter()
            .filter_map(|it| it.get("path").and_then(Value::as_str).map(String::from))
            .collect())
    } else if let Some(arr) = v.as_array() {
        Ok(arr
            .iter()
            .filter_map(|p| p.as_str().map(String::from))
            .collect())
    } else {
        Err("plan must be {would_trash:[…]} or a JSON array of paths".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_paths_and_apply() {
        let (p, apply) = parse(&a(&["/x", "--apply"])).unwrap();
        assert_eq!(p, a(&["/x"]));
        assert!(apply);
        assert!(!parse(&a(&["/x"])).unwrap().1);
        assert!(parse(&a(&["--apply"])).is_err());
    }

    #[test]
    fn dry_run_does_not_mutate() {
        let v = dry_run(&a(&["/no/such/file"]));
        assert_eq!(v["applied"], json!(false));
        assert_eq!(v["would_trash"][0]["exists"], json!(false));
    }

    #[test]
    fn trash_moves_file_to_trash() {
        // exercises the real adapter on macOS (Trash); the same path uses the Recycle Bin on Windows.
        let dir = std::env::temp_dir().join(format!("burrow_recycle_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("victim.txt");
        std::fs::write(&f, "x").unwrap();
        assert!(f.exists());

        let v = trash_all(&[f.to_string_lossy().into_owned()]);
        assert_eq!(v["applied"], json!(true));
        assert_eq!(v["trashed"][0]["ok"], json!(true), "trash failed: {v}");
        assert!(
            !f.exists(),
            "file should have been moved out to the Trash/Recycle Bin"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_paths_reads_a_held_preview() {
        // Scan once, execute the plan: a GUI holds a prior `trash` preview and
        // re-applies exactly it, without re-deriving the list.
        let preview = r#"{"applied":false,"would_trash":[{"path":"/a","exists":true},{"path":"/b","exists":true}]}"#;
        assert_eq!(plan_paths(preview).unwrap(), a(&["/a", "/b"]));
        // also accepts a bare array of paths
        assert_eq!(plan_paths(r#"["/c"]"#).unwrap(), a(&["/c"]));
    }

    #[test]
    fn trash_each_streams_one_result_per_path_in_order() {
        // Live progress: executing a plan emits a per-path event as each item
        // completes, so a GUI shows files trashed one by one — not a batch at the
        // end. Injectable deleter keeps the streaming logic pure to test.
        struct Fake;
        impl Delete for Fake {
            fn delete(&self, path: &str) -> Result<(), String> {
                if path == "/bad" {
                    Err("nope".into())
                } else {
                    Ok(())
                }
            }
        }
        let mut events = Vec::new();
        trash_each(&a(&["/x", "/bad", "/y"]), &Fake, |e| events.push(e));
        assert_eq!(events.len(), 3);
        assert_eq!(events[0]["path"], json!("/x"));
        assert_eq!(events[0]["ok"], json!(true));
        assert_eq!(
            events[1]["ok"],
            json!(false),
            "failed item: {:?}",
            events[1]
        );
        assert_eq!(events[2]["path"], json!("/y"));
    }
}
