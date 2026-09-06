//! Windows duplicate / similar-photo adapter via `czkawka_cli` (MIT core).
//!
//! On Windows, fclones cannot reflink-dedupe (no clonefile), so czkawka_cli is the duplicate
//! and similar-image engine. Czkawka writes version-dependent JSON to a file; this adapter
//! normalizes it into Burrow's stable, discovery-only model.

use crate::output::Failure;
use crate::platform;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scan {
    Duplicates,
    SimilarImages,
}

impl Scan {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Duplicates => "duplicates",
            Self::SimilarImages => "similar_images",
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct NormalizedFile {
    path: String,
    size: Option<u64>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct NormalizedGroup {
    file_count: usize,
    files: Vec<NormalizedFile>,
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
            out_json.into(),
        ],
        Scan::SimilarImages => vec![
            "image".into(),
            "-d".into(),
            dir.into(),
            "-s".into(),
            "5".into(), // similarity threshold (Phase 0 verified invocation)
            "-g".into(),
            "Gradient".into(), // perceptual hash algorithm
            "-c".into(),
            "16".into(),
            "--pretty-file-to-save".into(),
            out_json.into(),
        ],
    })
}

pub fn resolve_czkawka() -> Result<PathBuf, Failure> {
    if let Some(p) = platform::resolve_env_executable("BURROW_CZKAWKA", &["czkawka_cli"])? {
        return Ok(p);
    }
    if let Some(p) = platform::resolve_on_path(&["czkawka_cli"]) {
        return Ok(p);
    }
    Err(Failure::not_found(
        "czkawka_cli not found; install it or set BURROW_CZKAWKA",
    ))
}

/// A process-specific report path prevents concurrent Burrow processes from reading each
/// other's output. `execute` removes any stale file before launch and cleans up afterward.
pub fn report_path() -> PathBuf {
    std::env::temp_dir().join(format!("burrow-czkawka-{}.json", std::process::id()))
}

fn file_from_value(value: &Value) -> Option<NormalizedFile> {
    match value {
        Value::String(path) if !path.trim().is_empty() => Some(NormalizedFile {
            path: path.clone(),
            size: None,
        }),
        Value::Object(object) => {
            let path = object.get("path")?.as_str()?.to_string();
            if path.trim().is_empty() {
                return None;
            }
            let size = object.get("size").and_then(|size| {
                size.as_u64()
                    .or_else(|| size.as_str().and_then(|s| s.parse().ok()))
            });
            Some(NormalizedFile { path, size })
        }
        _ => None,
    }
}

fn collect_files(value: &Value, files: &mut Vec<NormalizedFile>) -> Result<(), Failure> {
    if let Some(file) = file_from_value(value) {
        files.push(file);
        return Ok(());
    }

    match value {
        Value::Array(values) => {
            for value in values {
                collect_files(value, files)?;
            }
        }
        _ => {
            return Err(Failure::invalid_output(
                "invalid czkawka output: group contains an entry without a file path",
            ))
        }
    }
    Ok(())
}

/// The groups of a report: its root array, one element per group.
///
/// The root must BE an array. This used to also accept an object wrapping the array under any
/// of four guessed keys (`groups`, `duplicates`, `similar_images`, `results`) — a tolerance for
/// report shapes nobody had captured, which is to say a guess about czkawka's format standing in
/// for knowledge of it. The fixture the integration suite drives writes an array; a real
/// `czkawka_cli` report has never been captured into this repo (see `STATUS.md`, "Windows sidecar
/// verification boundary"). When one is, its shape gets parsed here on evidence, not guessed at.
fn group_candidates(report: &Value) -> Result<Vec<&Value>, Failure> {
    let Value::Array(values) = report else {
        return Err(Failure::invalid_output(
            "invalid czkawka output: expected an array of groups at the root",
        ));
    };

    if values.iter().all(|value| file_from_value(value).is_some()) && !values.is_empty() {
        // Tolerate an older/flattened report containing one group directly at the root.
        Ok(vec![report])
    } else {
        Ok(values.iter().collect())
    }
}

fn redundant_bytes(files: &[NormalizedFile]) -> Option<u64> {
    let sizes: Option<Vec<u64>> = files.iter().map(|file| file.size).collect();
    let sizes = sizes?;
    let largest = sizes.iter().copied().max()?;
    sizes
        .into_iter()
        .try_fold(0u64, u64::checked_add)?
        .checked_sub(largest)
}

/// Convert version-dependent czkawka JSON into Burrow's stable discovery model.
pub fn normalize(scan: Scan, report: &Value) -> Result<Value, Failure> {
    let candidates = group_candidates(report)?;
    let mut groups = Vec::new();

    for candidate in candidates {
        let mut files = Vec::new();
        collect_files(candidate, &mut files)?;
        if files.len() >= 2 {
            groups.push(NormalizedGroup {
                file_count: files.len(),
                files,
            });
        } else {
            return Err(Failure::invalid_output(
                "invalid czkawka output: a group contains fewer than two files",
            ));
        }
    }

    let file_count = groups.iter().map(|group| group.file_count).sum::<usize>();
    let reclaimable = if scan == Scan::Duplicates {
        groups.iter().try_fold(0u64, |total, group| {
            total.checked_add(redundant_bytes(&group.files)?)
        })
    } else {
        None
    };

    // `ok` and `engine` are the ENVELOPE's — `data` does not restate them.
    Ok(json!({
        "scan": scan.as_str(),
        "available": true,
        "applied": false,
        "group_count": groups.len(),
        "file_count": file_count,
        "redundant_bytes": reclaimable,
        "groups": groups,
    }))
}

/// Run czkawka writing to `out_json`, then normalize that report. Czkawka uses exit code 11
/// to mean matching items were found, so both 0 and 11 are successful scan completions.
pub fn execute(
    scan: Scan,
    czkawka: &Path,
    args: &[String],
    out_json: &str,
) -> Result<Value, Failure> {
    match std::fs::remove_file(out_json) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(Failure::io(
                format!("failed to prepare czkawka report at {out_json}"),
                &e,
            ))
        }
    }

    let out = platform::command(czkawka, args)?
        .output()
        .map_err(|e| Failure::io("failed to run czkawka_cli", &e))?;
    if !out.status.success() && out.status.code() != Some(11) {
        let _ = std::fs::remove_file(out_json);
        return Err(Failure::process_failed(format!(
            "czkawka exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }

    let report = std::fs::read_to_string(out_json);
    let _ = std::fs::remove_file(out_json);
    let report = report.map_err(|e| {
        Failure::invalid_output(format!(
            "czkawka wrote no readable report at {out_json}: {e}"
        ))
    })?;
    let report = serde_json::from_str::<Value>(&report).map_err(|e| {
        Failure::invalid_output(format!(
            "invalid czkawka output: malformed JSON report: {e}"
        ))
    })?;
    normalize(scan, &report)
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
    fn similar_images_uses_the_verified_invocation() {
        // Phase 0 verified: `image -d <dir> -s 5 -g Gradient -c 16 --pretty-file-to-save out`.
        // -s (similarity) + -g (hash algorithm) pin match quality; dropping them silently
        // changes what counts as "similar".
        assert_eq!(
            plan(&Scan::SimilarImages, "C:/Pics", "sim.json").unwrap(),
            v(&[
                "image",
                "-d",
                "C:/Pics",
                "-s",
                "5",
                "-g",
                "Gradient",
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

    #[test]
    fn normalizes_duplicate_groups_and_reclaimable_bytes() {
        let raw = json!([
            [
                { "path": "C:/a.bin", "size": 100, "unknown": true },
                { "path": "C:/b.bin", "size": "100" }
            ],
            [
                { "path": "C:/c.bin", "size": 30 },
                { "path": "C:/d.bin", "size": 30 },
                { "path": "C:/e.bin", "size": 30 }
            ]
        ]);
        let report = normalize(Scan::Duplicates, &raw).unwrap();
        // `engine` and `ok` are the envelope's; the payload no longer restates them.
        assert!(report.get("engine").is_none(), "{report}");
        assert!(report.get("ok").is_none(), "{report}");
        assert_eq!(report["scan"], json!("duplicates"));
        assert_eq!(report["group_count"], json!(2));
        assert_eq!(report["file_count"], json!(5));
        assert_eq!(report["redundant_bytes"], json!(160));
        assert_eq!(report["groups"][0]["files"][0]["path"], json!("C:/a.bin"));
        assert_eq!(report["groups"][0]["files"][0]["size"], json!(100));
    }

    #[test]
    fn similar_images_and_missing_sizes_use_null_redundant_bytes() {
        let raw = json!([[
            { "path": "C:/a.jpg", "size": 100, "future_metadata": { "version": 10 } },
            { "path": "C:/b.jpg" }
        ]]);
        let report = normalize(Scan::SimilarImages, &raw).unwrap();
        assert_eq!(report["scan"], json!("similar_images"));
        assert!(report["redundant_bytes"].is_null());
        assert!(report["groups"][0]["files"][1]["size"].is_null());
    }

    #[test]
    fn reference_style_groups_are_flattened_without_panicking() {
        let raw = json!([[
            { "path": "C:/reference.bin", "size": 42 },
            [{ "path": "C:/copy.bin", "size": 42 }]
        ]]);
        let report = normalize(Scan::Duplicates, &raw).unwrap();
        assert_eq!(report["group_count"], json!(1));
        assert_eq!(report["file_count"], json!(2));
        assert_eq!(report["redundant_bytes"], json!(42));
    }

    #[test]
    fn empty_report_is_a_safe_empty_model() {
        let report = normalize(Scan::Duplicates, &json!([])).unwrap();
        assert_eq!(report["groups"], json!([]));
        assert_eq!(report["group_count"], json!(0));
        assert_eq!(report["file_count"], json!(0));
        assert_eq!(report["redundant_bytes"], json!(0));
    }

    #[test]
    fn unexpected_nonempty_report_is_an_error() {
        let err = normalize(Scan::Duplicates, &json!([{ "future": "shape" }])).unwrap_err();
        assert!(err.message.contains("invalid czkawka output"));
        assert_eq!(err.kind(), &crate::output::ErrorKind::InvalidOutput);
    }

    /// A report that is not a root array is not guessed at: the wrapper-key tolerance this
    /// replaced accepted four key names nobody had ever seen czkawka write.
    #[test]
    fn a_wrapped_report_is_invalid_output_not_a_guess() {
        for wrapped in [
            json!({ "groups": [[{ "path": "C:/a", "size": 1 }, { "path": "C:/b", "size": 1 }]] }),
            json!({ "results": [] }),
            json!("nope"),
        ] {
            let err = normalize(Scan::Duplicates, &wrapped).unwrap_err();
            assert_eq!(
                err.kind(),
                &crate::output::ErrorKind::InvalidOutput,
                "{wrapped}"
            );
        }
    }

    #[test]
    fn malformed_groups_and_metadata_strings_are_never_reported_as_files() {
        for report in [
            json!([{ "future": "shape", "format": "something" }]),
            json!([[{"path":"/a"}, {"path":"/b"}], [{"path": 42}]]),
            json!([[{"path":"/a"}, {"path":"/b"}], []]),
            json!([["", "/b"]]),
        ] {
            assert!(normalize(Scan::Duplicates, &report).is_err(), "{report}");
        }
    }
}
