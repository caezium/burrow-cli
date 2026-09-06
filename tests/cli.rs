//! Integration tests: run the built `burrow` binary against a fake engine dir
//! (testdata/engine) and assert the conductor's behavior end-to-end.

use std::process::Command;

fn burrow() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_burrow"));
    let engine = format!("{}/testdata/engine", env!("CARGO_MANIFEST_DIR"));
    c.env("BURROW_ENGINE_DIR", engine);
    c
}

fn burrow_no_engine() -> Command {
    Command::new(env!("CARGO_BIN_EXE_burrow"))
}

fn stdout(c: &mut Command) -> (bool, String) {
    let out = c.output().expect("run burrow");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn clean_defaults_to_dry_run() {
    let (ok, s) = stdout(burrow().arg("clean"));
    assert!(ok);
    assert!(s.contains(r#""invoked":["clean","--dry-run"]"#), "got: {s}");
    assert!(s.contains(r#""command":"clean""#));
}

#[test]
fn clean_apply_drops_dry_run() {
    let (_, s) = stdout(burrow().args(["clean", "--apply"]));
    assert!(s.contains(r#""invoked":["clean"]"#), "got: {s}");
}

#[test]
fn status_emits_envelope_with_engine_json() {
    let (ok, s) = stdout(burrow().arg("status"));
    assert!(ok);
    assert!(s.contains(r#""command":"status""#));
    assert!(s.contains(r#""health_score":92"#));
}

#[test]
fn status_watch_streams_raw_not_enveloped() {
    // --watch passes the engine's NDJSON straight through (no Burrow envelope).
    let (ok, s) = stdout(burrow().args(["status", "--watch"]));
    assert!(ok);
    assert!(s.contains("\"health_score\":92"), "got: {s}");
    assert!(!s.contains("burrow_cli"), "watch must stream raw: {s}");
}

#[test]
fn status_raw_is_passthrough() {
    let (_, s) = stdout(burrow().args(["status", "--raw"]));
    assert!(s.trim_start().starts_with('{'));
    assert!(!s.contains("burrow_cli"), "raw must not wrap: {s}");
}

#[test]
fn analyze_progress_streams_raw_ndjson_not_enveloped() {
    // --progress passes the engine's NDJSON scan-progress stream straight through
    // (no Burrow envelope), like status --watch — a GUI renders the treemap live.
    let (ok, s) = stdout(burrow().args(["analyze", "--progress", "/"]));
    assert!(ok, "got: {s}");
    assert!(
        s.contains(r#""type":"progress""#),
        "must stream progress: {s}"
    );
    assert!(
        s.contains(r#""type":"result""#),
        "must end with a result: {s}"
    );
    assert!(
        !s.contains("burrow_cli"),
        "progress must stream raw, not enveloped: {s}"
    );
}

#[test]
fn uninstall_without_app_fails() {
    let out = burrow().arg("uninstall").output().unwrap();
    assert!(!out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        s.contains(r#""error":"#),
        "failure should be enveloped: {s}"
    );
}

#[test]
fn missing_engine_dir_emits_error_envelope() {
    let missing =
        std::env::temp_dir().join(format!("burrow_missing_engine_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&missing);
    let out = burrow_no_engine()
        .env("BURROW_ENGINE_DIR", &missing)
        .arg("status")
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success());
    assert!(s.contains(r#""command":"status""#), "got: {s}");
    assert!(s.contains(r#""kind":"not_found""#), "got: {s}");
}

#[test]
fn missing_engine_executable_emits_error_envelope() {
    let dir = std::env::temp_dir().join(format!("burrow_empty_engine_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    let out = burrow_no_engine()
        .env("BURROW_ENGINE_DIR", &dir)
        .arg("status")
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success());
    assert!(s.contains(r#""command":"status""#), "got: {s}");
    assert!(s.contains(r#""kind":"not_found""#), "got: {s}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_command_fails() {
    let out = burrow().arg("bogus").output().unwrap();
    assert!(!out.status.success());
}

#[test]
fn failure_emits_error_envelope_on_stdout() {
    // A GUI reads stdout for every call; a command that fails via the conductor's
    // fail() path must emit the unified envelope (ok:false + command + error) on
    // stdout, not an empty stdout. `uninstall` with no app is a cross-platform
    // fail() case (see uninstall_without_app_fails).
    let out = burrow().arg("uninstall").output().unwrap();
    assert!(!out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        s.contains(r#""ok":false"#),
        "stdout must carry the error envelope: {s}"
    );
    assert!(s.contains(r#""command":"uninstall""#), "got: {s}");
    assert!(
        s.contains(r#""error""#),
        "envelope must carry an error: {s}"
    );
}

#[test]
fn version_prints() {
    let (ok, s) = stdout(burrow().arg("version"));
    assert!(ok);
    assert!(s.contains("burrow"));
}

// --- dupes (fclones sidecar) ---

fn burrow_fc() -> Command {
    let mut c = burrow();
    c.env(
        "BURROW_FCLONES",
        format!("{}/testdata/fclones", env!("CARGO_MANIFEST_DIR")),
    );
    c
}

#[test]
fn dupes_group_lists_groups() {
    let (ok, s) = stdout(burrow_fc().args(["dupes", "/tmp"]));
    assert!(ok);
    assert!(s.contains(r#""command":"dupes""#));
    assert!(s.contains("redundant_file_size"));
    assert!(s.contains(r#""files":["/tmp/a","/tmp/b"]"#), "got: {s}");
}

#[test]
fn missing_fclones_emits_error_envelope() {
    let missing =
        std::env::temp_dir().join(format!("burrow_missing_fclones_{}", std::process::id()));
    let _ = std::fs::remove_file(&missing);
    let out = burrow()
        .env("BURROW_FCLONES", &missing)
        .args(["dupes", "/tmp"])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success());
    assert!(s.contains(r#""command":"dupes""#), "got: {s}");
    assert!(s.contains(r#""kind":"not_found""#), "got: {s}");
}

#[test]
fn dupes_dedupe_preview_does_not_mutate() {
    let (_, s) = stdout(burrow_fc().args(["dupes", "dedupe", "/tmp"]));
    assert!(
        s.contains("redundant_file_size"),
        "preview shows the report"
    );
    assert!(
        !s.contains("\"deduped\":true"),
        "must NOT run the action without --apply: {s}"
    );
}

#[test]
fn dupes_dedupe_apply_runs_clonefile() {
    let (_, s) = stdout(burrow_fc().args(["dupes", "dedupe", "/tmp", "--apply"]));
    assert!(s.contains(r#""deduped":true"#), "got: {s}");
    assert!(s.contains(r#""action":"dedupe""#));
}

// --- rules engine (validate the shipped seed rules) ---

fn rules_dir() -> String {
    format!("{}/rules", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn seed_rules_all_validate() {
    let out = burrow()
        .args(["rules", "validate", &rules_dir()])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "seed rules must validate; got: {s}");
    assert!(s.contains(r#""problems":[]"#), "no problems expected: {s}");
}

#[test]
fn rules_list_includes_chrome_and_marks_risky() {
    let (ok, s) = stdout(burrow().args(["rules", "list", &rules_dir()]));
    assert!(ok);
    assert!(s.contains("Google Chrome"));
    assert!(s.contains("Xcode"));
    // chrome.history is risky -> default_selected must be false somewhere in output
    assert!(s.contains(r#""risk":"risky""#));
    assert!(s.contains(r#""default_selected":false"#));
}

#[test]
fn rules_dryrun_filters_by_app() {
    let (ok, s) = stdout(burrow().args([
        "rules",
        "dryrun",
        &rules_dir(),
        "--app",
        "com.google.Chrome",
    ]));
    assert!(ok);
    assert!(s.contains("Google Chrome"));
    assert!(!s.contains("Xcode"), "app filter should exclude Xcode: {s}");
    assert!(s.contains(r#""exists":"#)); // each item reports filesystem existence
}

// --- orphan scanner (filesystem) ---

#[test]
fn orphans_flags_only_unrelated_bundles() {
    use std::fs;
    let dir = std::env::temp_dir().join(format!("burrow_orphan_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("com.google.Chrome.savedState"), "x").unwrap(); // installed -> keep
    fs::write(dir.join("com.deadvendor.oldapp.savedState"), "x").unwrap(); // orphan
    fs::write(dir.join("com.apple.Safari.savedState"), "x").unwrap(); // safelisted
    fs::write(dir.join("my-notes.txt"), "x").unwrap(); // not bundle-shaped

    let (ok, s) = stdout(burrow().args([
        "orphans",
        dir.to_str().unwrap(),
        "--installed",
        "com.google.Chrome",
    ]));
    assert!(ok);
    assert!(
        s.contains("com.deadvendor.oldapp.savedState"),
        "flag orphan: {s}"
    );
    assert!(
        !s.contains("com.google.Chrome.savedState"),
        "installed not orphan: {s}"
    );
    assert!(!s.contains("com.apple.Safari"), "apple safelisted: {s}");
    assert!(!s.contains("my-notes"), "non-bundle not flagged: {s}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn orphans_envelope_keeps_public_fields_and_inventory_sources() {
    use std::fs;
    let dir = std::env::temp_dir().join(format!("burrow_orphan_contract_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("com.deadvendor.oldapp.savedState"), "x").unwrap();

    let (ok, s) = stdout(burrow().args([
        "orphans",
        dir.to_str().unwrap(),
        "--installed",
        "com.google.Chrome",
    ]));
    assert!(ok);
    assert!(s.contains(r#""roots":["#), "roots field missing: {s}");
    assert!(
        s.contains(r#""installed_count":1"#),
        "installed_count field missing: {s}"
    );
    assert!(
        s.contains(r#""inventory_sources":{"cli":1}"#),
        "inventory source count missing: {s}"
    );
    assert!(s.contains(r#""count":1"#), "count field missing: {s}");
    assert!(
        s.contains(r#""default_selected":false"#),
        "orphan hits must not be auto-selected: {s}"
    );
    assert!(
        s.contains("com.deadvendor.oldapp.savedState"),
        "expected orphan missing: {s}"
    );
    let _ = fs::remove_dir_all(&dir);
}

// --- tree-diff ---

#[test]
fn diff_baseline_then_growth() {
    use std::fs;
    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("burrow_diff_root_{pid}"));
    let scans = std::env::temp_dir().join(format!("burrow_diff_scans_{pid}"));
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&scans);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("small.txt"), "x").unwrap();

    let mut c = burrow();
    c.env("BURROW_SCAN_DIR", &scans)
        .args(["diff", root.to_str().unwrap()]);
    let (ok1, s1) = stdout(&mut c);
    assert!(ok1);
    assert!(
        s1.contains("baseline_saved"),
        "first run is a baseline: {s1}"
    );

    fs::write(root.join("big.bin"), vec![0u8; 200_000]).unwrap();
    let mut c2 = burrow();
    c2.env("BURROW_SCAN_DIR", &scans)
        .args(["diff", root.to_str().unwrap()]);
    let (ok2, s2) = stdout(&mut c2);
    assert!(ok2);
    assert!(s2.contains("\"changes\""), "second run diffs: {s2}");
    assert!(s2.contains("\"status\":\"grew\""), "shows growth: {s2}");

    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&scans);
}

// --- metrics history + digest (Phase 8) ---

#[test]
fn snapshot_then_digest() {
    let snap = std::env::temp_dir().join(format!("burrow_snapint_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&snap);

    let mut c = burrow();
    c.env("BURROW_SNAPSHOT_DIR", &snap).arg("snapshot");
    let (ok, s) = stdout(&mut c);
    assert!(ok, "snapshot: {s}");
    assert!(
        s.contains("\"health_score\":92"),
        "captured fixture health: {s}"
    );

    let mut c2 = burrow();
    c2.env("BURROW_SNAPSHOT_DIR", &snap).arg("digest");
    let (ok2, s2) = stdout(&mut c2);
    assert!(ok2);
    assert!(s2.contains("\"count\":1"), "digest counts the sample: {s2}");
    assert!(s2.contains("\"health_min\":92"));

    let _ = std::fs::remove_dir_all(&snap);
}

#[test]
fn sentinel_lists_trashed_apps() {
    use std::fs;
    let dir = std::env::temp_dir().join(format!("burrow_sent_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("Old App.app")).unwrap();
    fs::write(dir.join("receipt.txt"), "x").unwrap();
    let (ok, s) = stdout(burrow().args(["sentinel", dir.to_str().unwrap()]));
    assert!(ok);
    assert!(s.contains("Old App"), "lists trashed app: {s}");
    assert!(!s.contains("receipt"), "ignores non-apps: {s}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn net_emits_valid_envelope() {
    let (ok, s) = stdout(burrow().args(["net", "--limit", "5"]));
    assert!(s.contains(r#""command":"net""#));
    if cfg!(any(target_os = "macos", windows)) {
        assert!(ok, "net should run: {s}");
        assert!(s.contains("by_total_bytes"));
    } else {
        assert!(
            !ok,
            "unsupported platforms should fail with an envelope: {s}"
        );
        // Reconciled contract (burrow-cli#4 + #5): unsupported is signaled by the
        // structured error's `kind`, not a top-level `unsupported:true`.
        assert!(s.contains(r#""kind":"unsupported""#), "got: {s}");
    }
}

#[test]
fn trigger_reports_decision() {
    // fixture status has no disks -> disk_used_percent 0; threshold 0 -> should_clean true.
    let (ok, s) = stdout(burrow().args(["trigger", "--threshold", "0"]));
    assert!(ok);
    assert!(s.contains(r#""should_clean":true"#), "got: {s}");
    let (_, s2) = stdout(burrow().args(["trigger", "--threshold", "50"]));
    assert!(s2.contains(r#""should_clean":false"#), "got: {s2}");
}

// --- trash: scan once, execute the held plan ---

#[test]
fn trash_apply_plan_executes_a_held_plan_without_rescan() {
    use std::fs;
    let dir = std::env::temp_dir().join(format!("burrow_plan_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let victim = dir.join("v.txt");
    fs::write(&victim, "x").unwrap();
    // a held plan, exactly as a prior `trash` preview produces it
    let plan = dir.join("plan.json");
    fs::write(
        &plan,
        format!(
            r#"{{"would_trash":[{{"path":{:?}}}]}}"#,
            victim.to_string_lossy()
        ),
    )
    .unwrap();

    let out = burrow()
        .args(["trash", "--apply-plan", plan.to_str().unwrap()])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "got: {s}");
    assert!(s.contains(r#""applied":true"#), "got: {s}");
    assert!(!victim.exists(), "the planned file should be trashed: {s}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn trash_apply_plan_stream_emits_ndjson_per_item() {
    use std::fs;
    let dir = std::env::temp_dir().join(format!("burrow_stream_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let f1 = dir.join("a.txt");
    fs::write(&f1, "x").unwrap();
    let f2 = dir.join("b.txt");
    fs::write(&f2, "x").unwrap();
    let plan = dir.join("plan.json");
    fs::write(
        &plan,
        format!(
            r#"{{"would_trash":[{{"path":{:?}}},{{"path":{:?}}}]}}"#,
            f1.to_string_lossy(),
            f2.to_string_lossy()
        ),
    )
    .unwrap();

    let out = burrow()
        .args(["trash", "--apply-plan", plan.to_str().unwrap(), "--stream"])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    // one NDJSON event line per item — live progress, not one batch envelope
    let lines: Vec<&str> = s
        .lines()
        .filter(|l| l.trim_start().starts_with('{'))
        .collect();
    assert_eq!(lines.len(), 2, "expected one line per item, got: {s}");
    assert!(lines[0].contains(r#""ok":true"#), "got: {s}");
    assert!(
        !f1.exists() && !f2.exists(),
        "both planned files trashed: {s}"
    );
    let _ = fs::remove_dir_all(&dir);
}
