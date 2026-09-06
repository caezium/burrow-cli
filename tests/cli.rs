//! Integration tests: run the built `burrow` binary against a fake engine dir
//! (testdata/engine) and assert the conductor's behavior end-to-end.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static FIXTURE_ID: AtomicUsize = AtomicUsize::new(0);

fn burrow() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_burrow"));
    let engine = format!("{}/testdata/engine", env!("CARGO_MANIFEST_DIR"));
    c.env_remove("BURROW_ENGINE");
    c.env("BURROW_ENGINE_DIR", engine);
    c.env("BURROW_TELEMETRY", "0");
    c.env(
        "BURROW_CONFIG_DIR",
        std::env::temp_dir().join(format!("burrow_test_config_{}", std::process::id())),
    );
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

fn fake_executable(prefix: &str, windows_script: &str, unix_script: &str) -> (PathBuf, PathBuf) {
    let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("burrow_{prefix}_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(if cfg!(windows) {
        format!("{prefix}.cmd")
    } else {
        prefix.to_string()
    });
    let script = if cfg!(windows) {
        windows_script
    } else {
        unix_script
    };
    std::fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    (root, path)
}

fn fake_czkawka() -> (PathBuf, PathBuf) {
    fake_executable(
        "czkawka_cli",
        "@echo off\r\nset \"out=\"\r\n:loop\r\nif \"%~1\"==\"\" goto done\r\nset \"out=%~1\"\r\nshift\r\ngoto loop\r\n:done\r\n> \"%out%\" echo [[{\"path\":\"C:/fixture/a.bin\",\"size\":64},{\"path\":\"C:/fixture/b.bin\",\"size\":64}]]\r\nexit /b 11\r\n",
        "#!/bin/sh\nout=\"\"\nfor arg in \"$@\"; do out=\"$arg\"; done\nprintf '%s\\n' '[[{\"path\":\"C:/fixture/a.bin\",\"size\":64},{\"path\":\"C:/fixture/b.bin\",\"size\":64}]]' > \"$out\"\nexit 11\n",
    )
}

fn fake_bcu(stderr: &str, exit_code: i32) -> (PathBuf, PathBuf) {
    let windows_script = if stderr.is_empty() {
        "@echo off\r\necho fake BCU success\r\nexit /b 0\r\n".to_string()
    } else {
        format!("@echo off\r\necho {stderr} 1>&2\r\nexit /b {exit_code}\r\n")
    };
    let unix_script = if stderr.is_empty() {
        "#!/bin/sh\nprintf '%s\\n' 'fake BCU success'\nexit 0\n".to_string()
    } else {
        format!("#!/bin/sh\nprintf '%s\\n' '{stderr}' >&2\nexit {exit_code}\n")
    };
    fake_executable("BCU-console", &windows_script, &unix_script)
}

#[test]
fn clean_defaults_to_dry_run() {
    let (ok, s) = stdout(burrow().arg("clean"));
    assert!(ok);
    assert!(s.contains(r#""invoked":["clean","--dry-run"]"#), "got: {s}");
    assert!(s.contains(r#""command":"clean""#));
}

/// The inversion, end to end and in the direction that destroys data.
///
/// `mo clean` runs LIVE and `--dry-run` previews it; `burrow-engine clean` previews and
/// `--apply` runs it. The conductor's own contract is the engine's, so `--apply` has to REACH
/// the engine — it used to be stripped, and stripping it against this engine turns every real
/// clean into a no-op. The pair is asserted together, because each half is the other's failure:
/// a preview that carries `--apply` deletes what the caller asked to be shown, and a live run
/// that carries `--dry-run` silently does nothing.
#[test]
fn clean_apply_reaches_the_engine_and_the_preview_states_dry_run() {
    let (_, live) = stdout(burrow().args(["clean", "--apply"]));
    assert!(
        live.contains(r#""invoked":["clean","--apply"]"#),
        "got: {live}"
    );
    assert!(!live.contains("--dry-run"), "must not send both: {live}");

    let (_, preview) = stdout(burrow().arg("clean"));
    assert!(
        preview.contains(r#""invoked":["clean","--dry-run"]"#),
        "got: {preview}"
    );
    assert!(
        !preview.contains("--apply"),
        "preview must not apply: {preview}"
    );
}

#[test]
fn clean_stream_forwards_engine_output_raw_not_enveloped() {
    // --stream inherits the engine's stdio so a GUI reads its live NDJSON line-by-line instead
    // of one buffered envelope. There is no envelope to unwrap here — the engine emits none for
    // a stream — so the lines pass straight through.
    let (ok, s) = stdout(burrow().args(["clean", "--stream"]));
    assert!(ok, "got: {s}");
    assert!(
        s.contains(r#""event":"would_remove""#),
        "engine NDJSON must pass through: {s}"
    );
    // `--stream` must reach the engine (it is a real engine flag on clean/optimize), and the
    // dry-run guard must survive alongside it.
    assert!(
        s.contains(r#""invoked":["clean","--stream","--dry-run"]"#),
        "got: {s}"
    );
    assert!(
        !s.contains("burrow_cli"),
        "stream must be raw, not enveloped: {s}"
    );
}

/// `--stream` is a real engine flag on `clean`/`optimize`/`purge` ONLY; the engine refuses it
/// anywhere else rather than accepting and ignoring it. The conductor must preserve that
/// refusal so a caller waiting for progress never receives a snapshot as a successful stream.
#[test]
fn unsupported_stream_flags_reach_the_engine_and_are_refused() {
    for cmd in ["installer", "history", "status"] {
        let (ok, s) = stdout(burrow().args([cmd, "--stream"]));
        assert!(!ok, "got: {s}");
        let value: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(value["ok"], false);
        assert!(value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("--stream"));
    }
}

/// `purge --stream` is the engine's BUR-132 stream, in `clean --stream`'s vocabulary. Both halves
/// pass through raw: the preview's `would_remove` frames and `dry_run:true` `done`, and the live
/// run's `removed` frames and `freed_*` `done`. Every frame arrives verbatim — no envelope, no
/// re-wrapping, nothing added or reordered.
#[test]
fn purge_stream_forwards_both_halves_raw_frame_for_frame() {
    let (ok, s) = stdout(burrow().args(["purge", "--stream"]));
    assert!(ok, "got: {s}");
    assert_eq!(
        s.lines().collect::<Vec<_>>(),
        [
            r#"{"event":"would_remove","path":"/fixture/Caches/A","bytes":4}"#,
            r#"{"event":"would_remove","path":"/fixture/Caches/B","bytes":8}"#,
            r#"{"event":"done","dry_run":true,"would_free_bytes":12,"would_free_human":"12 B","count":2,"invoked":["purge","--stream","--dry-run"]}"#,
        ]
    );

    let (ok, s) = stdout(burrow().args(["purge", "--stream", "--apply"]));
    assert!(ok, "got: {s}");
    assert_eq!(
        s.lines().collect::<Vec<_>>(),
        [
            r#"{"event":"removed","path":"/fixture/Caches/A","bytes":4}"#,
            r#"{"event":"removed","path":"/fixture/Caches/B","bytes":8}"#,
            r#"{"event":"done","freed_bytes":12,"freed_human":"12 B","moved_to_trash_bytes":12,"moved_to_trash_human":"12 B","removed":2,"failed":0,"protected":0,"invoked":["purge","--stream","--apply"]}"#,
        ]
    );
}

/// `clean --apply --plan <file>` executes exactly a held preview. `--plan` and its file are the
/// engine's arguments and reach it untouched, with the conductor's apply/dry-run translation
/// stated after them exactly as for any other `clean`.
#[test]
fn clean_plan_reaches_the_engine_with_its_file() {
    let (ok, s) = stdout(burrow().args(["clean", "--apply", "--plan", "/x/plan.json"]));
    assert!(ok, "got: {s}");
    assert!(
        s.contains(r#""invoked":["clean","--plan","/x/plan.json","--apply"]"#),
        "got: {s}"
    );
}

/// `uninstall --list` is the one engine command that emits no envelope — a bare JSON array.
/// It has nothing to strip, so it becomes the payload and the conductor wraps it as `data`.
#[test]
fn uninstall_list_wraps_the_engines_bare_array() {
    let (ok, s) = stdout(burrow().args(["uninstall", "--list"]));
    assert!(ok, "got: {s}");
    assert!(s.contains(r#""command":"uninstall""#), "got: {s}");
    assert!(s.contains(r#""data":[{"name":"Fixture""#), "got: {s}");
    // --list takes no app name and is never qualified by an apply/dry-run flag.
    assert!(!s.contains("--dry-run"), "got: {s}");
}

#[test]
fn status_emits_envelope_with_engine_json() {
    let (ok, s) = stdout(burrow().arg("status"));
    assert!(ok);
    assert!(s.contains(r#""command":"status""#));
    assert!(s.contains(r#""health_score":92"#));
}

/// `status --watch [--interval <secs>]` is the engine's BUR-132 stream: one line per tick, each
/// the buffered `status` `data` object, no envelope. The conductor inherits the engine's stdio,
/// so every tick reaches the caller as it is written — there is no conductor buffer that could
/// hold the frames until the watch ends. `--interval` and its value ride through as the engine's
/// own arguments; `--raw` is the conductor's and never reaches it.
#[test]
fn status_watch_forwards_every_tick_raw_and_verbatim() {
    let out = burrow()
        .args(["status", "--watch", "--interval", "0.5", "--raw"])
        .env("BURROW_WATCH_FRAMES", "3")
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "got: {s}");
    let frame = r#"{"collected_at":"2026-06-25T10:30:00Z","host":"fixture","health_score":92,"cpu":{"usage":12.4},"invoked":["status","--watch","--interval","0.5"]}"#;
    assert_eq!(s.lines().collect::<Vec<_>>(), [frame, frame, frame]);
    assert!(
        !s.contains("burrow_cli"),
        "a tick is the data object, not an envelope: {s}"
    );
}

/// `analyze --progress <path>` is the engine's BUR-132 stream: `{"type":"progress",…}` running
/// totals as each top-level directory finishes, then one `{"type":"result","data":…}` whose
/// `data` is the buffered command's payload. Raw, in order, nothing wrapped.
#[test]
fn analyze_progress_forwards_progress_frames_then_the_result_raw() {
    let root = format!("{}/testdata", env!("CARGO_MANIFEST_DIR"));
    let (ok, s) = stdout(burrow().args(["analyze", "--progress", &root]));
    assert!(ok, "got: {s}");
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines.len(), 3, "got: {s}");
    assert!(
        lines[0].starts_with(r#"{"type":"progress","files":10,"dirs":2,"bytes":4096,"path":"#),
        "got: {}",
        lines[0]
    );
    assert!(
        lines[1].starts_with(r#"{"type":"progress","files":20,"dirs":4,"bytes":8192,"path":"#),
        "got: {}",
        lines[1]
    );
    assert!(
        lines[2].starts_with(r#"{"type":"result","data":{"path":"#),
        "got: {}",
        lines[2]
    );
    assert!(
        lines[2].ends_with(r#""total_size":8192}}"#),
        "got: {}",
        lines[2]
    );
    assert!(!s.contains("burrow_cli"), "stream must be raw: {s}");
}

/// A stream's exit code is the engine's, relayed as-is, and its last line is whatever the engine
/// wrote — the conductor neither swallows a failing stream into `ok:true` nor wraps its tail in
/// a second envelope. `analyze --progress` on a path that does not exist is the engine's own
/// failing stream: the buffered command's error envelope as the final line, exit 1.
#[test]
fn a_failing_stream_relays_the_engines_exit_code_and_last_line_untouched() {
    let missing = format!(
        "{}/testdata/definitely-not-here",
        env!("CARGO_MANIFEST_DIR")
    );
    let out = burrow()
        .args(["analyze", "--progress", &missing])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "got: {s}");
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines.len(), 1, "the engine's last line, and only it: {s}");
    assert!(lines[0].starts_with(r#"{"ok":false,"#), "got: {s}");
    assert!(lines[0].contains("definitely-not-here"), "got: {s}");
    // Exactly one envelope: the engine's. A conductor that captured the stream and re-wrapped
    // it would print `burrow_cli` twice.
    assert_eq!(s.matches("burrow_cli").count(), 1, "got: {s}");
}

/// The engine still refuses the flags it does not implement — `--watch-interval` is the
/// digger's spelling, kept refused in favour of `--interval <secs>` — and the conductor forwards
/// them and lets the refusal through, rather than dropping the flag and running a one-shot.
/// That would hand a caller who asked for a cadence a single snapshot and no indication anything
/// was different: the same accept-and-ignore bug one layer up. Because the flag is not on the
/// row's `engine_streams`, this is NOT a passthrough: the refusal is captured and relayed with
/// the engine's classification, like any other failure.
#[test]
fn a_flag_the_engine_does_not_implement_is_refused_not_silently_dropped() {
    for args in [
        ["status", "--watch-interval", "5s"].as_slice(),
        ["status", "--interval", "5"].as_slice(),
        ["history", "--watch"].as_slice(),
        ["clean", "--progress"].as_slice(),
    ] {
        let out = burrow().args(args).output().unwrap();
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(!out.status.success(), "{args:?} must fail: {s}");
        assert!(s.contains(r#""ok":false"#), "{args:?}: {s}");
        // The reason comes off the ENGINE's envelope. The engine writes it to stdout and leaves
        // stderr empty, so a conductor reading stderr would report a blank reason.
        assert!(
            s.contains(args[1]),
            "{args:?} must name the refused flag: {s}"
        );
        // Relayed through the conductor's envelope, which names the conductor's version — not
        // printed raw as the engine's.
        assert!(
            s.contains(&format!(r#""burrow_cli":"{}""#, env!("CARGO_PKG_VERSION"))),
            "{args:?} must be the conductor's envelope: {s}"
        );
    }
}

/// `--raw` asks for the payload without the conductor's envelope. The ENGINE refuses the flag
/// outright (it has exactly one output mode), so the conductor must consume it rather than
/// forward it — forwarding turns every `--raw` invocation into an exit-2 failure.
#[test]
fn status_raw_is_the_unwrapped_payload_and_never_reaches_the_engine() {
    let (ok, s) = stdout(burrow().args(["status", "--raw"]));
    assert!(ok, "got: {s}");
    assert!(s.trim_start().starts_with('{'));
    assert!(!s.contains("burrow_cli"), "raw must not wrap: {s}");
    assert!(s.contains(r#""health_score":92"#), "got: {s}");
    assert!(!s.contains("--raw"), "--raw must not reach the engine: {s}");
}

/// The output contract. The engine already emits `{ok, burrow_cli, engine, command, data}`, so
/// the conductor could forward it or nest it; it UNWRAPS and re-wraps instead. The observable
/// consequence asserted here is the one that decides it: `burrow_cli` names the CONDUCTOR's
/// version, not the engine's, and `data` is the payload rather than a second envelope.
#[test]
fn the_conductor_re_wraps_rather_than_forwarding_the_engines_envelope() {
    let (ok, s) = stdout(burrow().arg("status"));
    assert!(ok, "got: {s}");
    let v: serde_json::Value = serde_json::from_str(s.trim()).expect("one envelope");
    assert_eq!(v["ok"], serde_json::json!(true));
    assert_eq!(
        v["burrow_cli"],
        serde_json::json!(env!("CARGO_PKG_VERSION"))
    );
    assert_ne!(
        v["burrow_cli"],
        serde_json::json!("0.1.0"),
        "the fixture engine's own version must not be reported as the conductor's: {s}"
    );
    // `data` is the payload, not a nested envelope — the app's BurrowEnvelope.parse hands these
    // bytes straight to MoleStatus's decoder.
    assert_eq!(v["data"]["health_score"], serde_json::json!(92));
    assert!(
        v["data"]["ok"].is_null(),
        "data must not be an envelope: {s}"
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

/// A stand-in engine that replays one of the vendored REAL engine captures
/// (`testdata/engine-captures/`) byte-for-byte on stdout and exits with `code`, which is what the
/// engine itself does for a classified refusal. Nothing about the envelope is retyped here — the
/// script only prints a file whose contents came off a real `burrow-engine` run.
///
/// The exit code is a parameter rather than a fixed 1 because the engine does not use one code:
/// a malformed-argv `error` exits 2 (`evict-error.json`), a classified refusal exits 1. Both are
/// recorded per capture in `PROVENANCE.txt`, and a replay that got the code wrong would be
/// testing a process this engine never produces.
///
/// # The path is built with `Path::join`, and that is load-bearing on Windows
///
/// It is interpolated into a `.cmd` and read by cmd's BUILT-IN `type`, which parses the filename
/// itself instead of handing it to the Win32 API — and built-ins take `/` as the switch
/// character, so they do not reliably open a path that contains one even quoted, while
/// `CreateFile` would. `format!("{}/testdata/…", env!("CARGO_MANIFEST_DIR"))` produced exactly
/// that on Windows and nowhere else: `D:\a\burrow-cli\burrow-cli/testdata/engine-captures/….json`.
///
/// The failure was silent, which is what made it read as a conductor bug. `type` reports a name
/// it cannot open on STDERR and `engine::execute` reads only stdout, so the stub printed nothing,
/// `exit /b` still returned the code the caller asked for, and the conductor — seeing a non-zero
/// exit and no envelope to relay — wrote its own `process_failed` about an engine that had in
/// fact never replayed anything.
///
/// `Path::join` emits the host's own separator, so each half of the script gets a path the shell
/// running it can open. Do not flatten this back into a `format!` with a literal `/`.
fn replaying_engine(capture: &str, code: i32) -> (PathBuf, serde_json::Value) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("engine-captures")
        .join(capture);
    let text = std::fs::read_to_string(&path).expect("vendored capture must exist");
    let parsed: serde_json::Value = serde_json::from_str(text.trim()).expect("capture is JSON");
    let (root, _) = fake_executable(
        "burrow-engine",
        &format!(
            "@echo off\r\ntype \"{}\"\r\nexit /b {code}\r\n",
            path.display()
        ),
        &format!("#!/bin/sh\ncat \"{}\"\nexit {code}\n", path.display()),
    );
    (root, parsed)
}

/// `evict` and `sentinel` reach the engine, on whatever platform this is.
///
/// These are the two `engine::windows_refusal` used to answer for, and answering for them was
/// always meant to be temporary — the engine returned `ok:true, count:0` about a Trash it had
/// never opened, and an `ok:true` eviction preview whose every item said `supported:false`, so
/// the conductor refused before spawning anything. `burrow-engine 5e42bf0` fixed both at the
/// source and the table came out, which means the conductor now has to actually run the engine
/// for these two and hand back what it says.
///
/// Run on every platform because that is the claim: there is no platform on which the conductor
/// pre-empts these. On Windows it is also the regression test — before the table came out, this
/// asserted-for message never appeared, because no process was ever started.
///
/// Each capture is a REAL failure of the command it is replayed for, so the envelope that comes
/// back is coherent rather than a `net` refusal wearing another command's name. Between them they
/// cover two of the engine's three classifications (`error` and `not_found`); `unsupported` is
/// already covered by `the_engines_classification_survives_a_reworded_message` above.
#[test]
fn the_two_commands_that_used_to_be_pre_empted_now_reach_the_engine() {
    for (command, capture, code) in [
        ("evict", "evict-error.json", 2),
        ("sentinel", "sentinel-not-found.json", 1),
    ] {
        let (root, expected) = replaying_engine(capture, code);
        let out = burrow_no_engine()
            .env("BURROW_ENGINE_DIR", &root)
            .arg(command)
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&out.stdout);
        let v: serde_json::Value = serde_json::from_str(s.trim())
            .unwrap_or_else(|e| panic!("{command}: one envelope ({e}): {s}"));

        assert!(!out.status.success(), "{command}: {s}");
        assert_eq!(v["ok"], serde_json::json!(false), "{command}: {s}");
        // The message is the proof the engine RAN: the conductor has no way to write this
        // sentence, and before `5e42bf0` it would have written its own instead.
        assert_eq!(
            v["error"]["message"], expected["error"]["message"],
            "{command}: the engine's own reason must reach the caller, not a conductor refusal: {s}"
        );
        assert_eq!(
            v["error"]["kind"], expected["error"]["kind"],
            "{command}: and its own classification with it: {s}"
        );
        assert_eq!(v["command"], serde_json::json!(command), "{command}: {s}");
        // No top-level `feature`. README documents that key as how a caller tells a conductor's
        // own refusal from a relayed engine one, so its ABSENCE is the machine-readable form of
        // "this conductor did not answer for the engine".
        assert!(
            v.get("feature").is_none(),
            "{command}: a relayed engine failure is not a conductor refusal: {s}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// The regression this whole path exists for, end to end through the real `burrow` binary.
///
/// The capture is a real engine refusal whose message was REWORDED — an ordinary, non-breaking
/// edit in a separate repo — so it contains none of the words the conductor's wording table
/// looks for, while the engine still classifies it `unsupported`. Before the relay, the
/// conductor re-derived the kind from the prose and answered `error`, and anything filtering on
/// `unsupported` stopped seeing the refusal. The expected kind is read out of the capture, so
/// this asserts "whatever the engine said", not a constant that could drift away from it.
#[test]
fn the_engines_classification_survives_a_reworded_message() {
    let (root, capture) = replaying_engine("net-unsupported-reworded.json", 1);
    let out = burrow_no_engine()
        .env("BURROW_ENGINE_DIR", &root)
        .arg("net")
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(s.trim()).expect("one envelope: {s}");

    assert!(!out.status.success());
    assert_eq!(v["ok"], serde_json::json!(false), "got: {s}");
    assert_eq!(
        v["error"]["kind"], capture["error"]["kind"],
        "the engine's own kind must reach the caller: {s}"
    );
    assert_eq!(
        v["error"]["message"], capture["error"]["message"],
        "got: {s}"
    );
    // Unchanged by the relay: the version names the CONDUCTOR, and the engine's `feature` key —
    // README's marker for a refusal the conductor raised itself — does not cross over.
    assert_eq!(
        v["burrow_cli"],
        serde_json::json!(env!("CARGO_PKG_VERSION"))
    );
    assert_ne!(v["burrow_cli"], capture["burrow_cli"], "got: {s}");
    assert!(capture.get("feature").is_some(), "capture must have one");
    assert!(v.get("feature").is_none(), "must not be relayed: {s}");
    let _ = std::fs::remove_dir_all(&root);
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

#[cfg(not(windows))]
#[test]
fn sentinel_watch_streams_ndjson_events_and_exits_at_max_ticks() {
    use std::fs;
    let dir = std::env::temp_dir().join(format!("burrow_sentwatch_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("Foo.app")).unwrap();
    // Watch mode: NDJSON event stream (raw, no envelope — like status --watch). The first
    // tick announces the CURRENT trash state (a login-item starting should surface what's
    // already actionable), then each tick announces new arrivals. --max-ticks bounds the
    // loop so daemons are testable + cron-able.
    let (ok, s) = stdout(burrow().args([
        "sentinel",
        dir.to_str().unwrap(),
        "--watch",
        "--interval-ms",
        "10",
        "--max-ticks",
        "2",
    ]));
    assert!(ok, "got: {s}");
    assert!(
        s.contains(r#""event":"trashed_app""#) && s.contains(r#""name":"Foo""#),
        "must announce the trashed app: {s}"
    );
    assert!(!s.contains("burrow_cli"), "watch streams raw NDJSON: {s}");
    let _ = fs::remove_dir_all(&dir);
}

/// `--watch` with an interval and NO directory must fall back to `<home>/.Trash` — never to the
/// interval's VALUE.
///
/// This is the argv the shipped `packaging/launchd/dev.caezium.burrow.sentinel.plist.template`
/// runs (`sentinel --watch --interval-ms 15000`). The conductor picked the first token that did
/// not start with `--`, which is `15000`, so the installed agent polled a relative directory of
/// that name — a directory that does not exist — and announced nothing for as long as it ran.
/// HOME is redirected so the fallback lands somewhere this test controls instead of the real
/// Trash.
#[cfg(not(windows))]
#[test]
fn sentinel_watch_infers_the_home_trash_and_never_a_flags_value() {
    use std::fs;
    let home = std::env::temp_dir().join(format!("burrow_sentinferred_{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(home.join(".Trash").join("Foo.app")).unwrap();
    let (ok, s) = stdout(burrow().env("HOME", &home).args([
        "sentinel",
        "--watch",
        "--interval-ms",
        "10",
        "--max-ticks",
        "1",
    ]));
    assert!(ok, "got: {s}");
    assert!(
        s.contains(r#""name":"Foo""#),
        "the inferred <home>/.Trash must be what is watched, not `10`: {s}"
    );
    let _ = fs::remove_dir_all(&home);
}

/// The same defect on the other two commands that take a value-taking flag: the flag's value
/// stood in for the thing the command acts on.
///
/// `slim --output thin.bin` thinned `thin.bin` — the file it was told to WRITE — and
/// `win-uninstall --confidence Good` uninstalled an app named `Good`. With no positional present
/// at all, the only correct answer is the "needs an argument" refusal; taking the value instead
/// got a plausible-looking run against the wrong target.
#[test]
fn a_value_taking_flags_value_is_never_the_positional_argument() {
    let out = burrow()
        .args(["slim", "--output", "/tmp/burrow_slim_output_probe"])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "got: {s}");
    assert!(
        s.contains("needs a path to a Mach-O binary"),
        "--output's value is the destination, not the input binary: {s}"
    );

    let out = burrow()
        .args(["win-uninstall", "--confidence", "Good"])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "got: {s}");
    assert!(
        s.contains("needs an app name"),
        "--confidence's value is a grade, not the app to remove: {s}"
    );
}

// --- the overlapping commands, after they moved to the engine ---
//
// `dupes rules orphans net evict photos slim-check sentinel` were implemented BOTH here and in
// the engine. The conductor's implementations are deleted, so what is left to test here is the
// conductor's half of the contract — that each one is dispatched to the engine with its arguments
// intact, and that the envelope names who actually answered. The RESULTS are the engine's
// contract and are tested in the engine's own suite; asserting them here would only pin this
// repo to a fixture's invention of them.
//
// The tests these replaced asserted the deleted implementations' output shapes (orphan bundle-id
// grading, fclones group parsing, rule dryrun selection, nettop row ranking). That coverage moved
// with the code rather than disappearing — see the engine's suite.

/// Every overlapping command reaches the engine, carrying its own arguments unchanged.
///
/// `data.invoked` is the fixture echoing back its argv, so this asserts the exact wire form
/// rather than "something happened".
#[test]
fn overlapping_commands_dispatch_to_the_engine_with_their_arguments() {
    let cases: &[(&[&str], &str)] = &[
        (
            &["photos", "/tmp", "--threshold", "8"],
            r#""photos","/tmp","--threshold","8""#,
        ),
        (
            &["rules", "dryrun", "/tmp", "--app", "com.x.y"],
            r#""rules","dryrun","/tmp","--app","com.x.y""#,
        ),
        (&["net", "--limit", "3"], r#""net","--limit","3""#),
        (
            &["orphans", "/tmp", "--installed", "a,b"],
            r#""orphans","/tmp","--installed","a,b""#,
        ),
        (&["slim-check", "/bin/ls"], r#""slim-check","/bin/ls""#),
        (&["dupes", "group", "/tmp"], r#""dupes","group","/tmp""#),
        (&["evict", "/tmp/x"], r#""evict","/tmp/x""#),
    ];
    for (args, expected) in cases {
        let (ok, s) = stdout(burrow().args(*args));
        assert!(ok, "{args:?} must reach the engine: {s}");
        assert!(s.contains(expected), "{args:?} argv mangled: {s}");
    }
}

/// A user's argument reaches the engine as ONE argument, byte for byte, with no shell reading it
/// on the way.
///
/// `platform::command` used to run `.cmd`/`.bat` targets through `cmd /C <path> <args…>` on
/// Windows, unescaped — so once every user positional started flowing through it (`analyze
/// <path>`, `uninstall <app>`, `--installed a,b`), an `&` in an app name was a command separator
/// to `cmd`, not three bytes of the name. The wrapper is gone and the target is spawned directly;
/// Rust's own `Command` quotes batch-file arguments (CVE-2024-24576) where a shim is still in play.
///
/// Asserted through the fixture engine's `data.invoked` echo, parsed rather than substring-matched,
/// because the Windows half of the fixture serialises through PowerShell's `ConvertTo-Json`, which
/// writes `&` as `\u0026` — the same string, spelled differently.
#[test]
fn a_users_argument_reaches_the_engine_verbatim_and_no_shell_reads_it() {
    let hostile = "Foo & echo pwned";
    for command in ["analyze", "uninstall"] {
        let out = burrow().args([command, hostile]).output().unwrap();
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{command}: {s}");
        let v: serde_json::Value = serde_json::from_str(s.trim())
            .unwrap_or_else(|e| panic!("{command}: one envelope ({e}): {s}"));
        let invoked = v["data"]["invoked"]
            .as_array()
            .unwrap_or_else(|| panic!("{command}: fixture must echo argv: {s}"));
        assert!(
            invoked.iter().any(|a| a == hostile),
            "{command}: the argument must arrive as one verbatim argv entry: {s}"
        );
        // Had a shell parsed it, `echo pwned` would have run and its output would be on stdout
        // as a line of its own, outside the envelope.
        assert!(
            !s.lines().any(|l| l.trim() == "pwned"),
            "{command}: a shell ran part of the argument: {s}"
        );
    }
}

/// The `engine` field must name the engine, not `native`.
///
/// It reported `native` for these commands until they moved, and `fclones` for `dupes` — both
/// true when this conductor ran them itself and both wrong now. A stale value here is the kind
/// of bug nothing else catches: the answer is correct, only its attribution lies.
#[test]
fn the_envelope_credits_the_engine_for_commands_it_now_serves() {
    for cmd in ["photos", "rules", "net", "orphans", "slim-check", "dupes"] {
        let (ok, s) = stdout(burrow().args([cmd, "/tmp"]));
        // Attribution on a FAILED run proves nothing: a refusal envelope names the engine too, so
        // dropping the status here would let this pass while every command was erroring out.
        assert!(
            ok,
            "{cmd} must succeed before its attribution means anything: {s}"
        );
        assert!(
            s.contains(r#""engine":"burrow-engine""#),
            "{cmd} must credit the engine: {s}"
        );
        assert!(
            !s.contains(r#""engine":"native""#) && !s.contains(r#""engine":"fclones""#),
            "{cmd} still claims it answered itself: {s}"
        );
    }
}

/// The other half of the attribution contract: a command must name the SAME engine whether it
/// worked or not.
///
/// The failure envelopes hardcoded `burrow-engine`, so every command the conductor serves itself
/// answered with two different engines depending on the outcome — `slim` said `native` when it
/// worked and `burrow-engine` when it did not, `win-dupes` said `czkawka` and then
/// `burrow-engine`, `win-uninstall` said `bcu` and then `burrow-engine`. The success-path test
/// above could not see any of it, which is why the bug survived: attribution was only ever
/// asserted on the half that was right.
///
/// The PAIR is the claim, and it is read out of two real envelopes rather than restated. Pinning
/// "`slim` fails as `native`" on its own would go on passing if the success path drifted later,
/// and the two halves lying in the same direction is precisely what a caller routing on this
/// field cannot survive. The expected value is named too, so an `engine_for` collapsed to a
/// single constant could not satisfy the pairing trivially.
#[test]
fn a_commands_failure_names_the_same_engine_as_its_success() {
    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("burrow_pairing_root_{pid}"));
    let scans = std::env::temp_dir().join(format!("burrow_pairing_scans_{pid}"));
    // Set for BOTH sidecars, so a machine with a real czkawka or BCU installed runs the same
    // paths as one without: the preview branch, which resolves nothing.
    let absent = std::env::temp_dir().join(format!("burrow_pairing_absent_{pid}"));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&scans);
    let _ = std::fs::remove_file(&absent);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.txt"), "x").unwrap();
    let dir = root.to_str().unwrap();

    // (the engine that serves it, an invocation that SUCCEEDS, one that FAILS)
    let mut cases: Vec<(&str, Vec<&str>, Vec<&str>)> = vec![
        // The conductor's own code: a tree scan, or a refusal for want of a directory to scan.
        ("native", vec!["diff", dir], vec!["diff"]),
        // BCU. The preview names the command it WOULD run without resolving anything, so it
        // succeeds on every platform; the failure is `--confidence`'s value being read as the
        // app name. Both halves say `bcu` even though no BCU exists here — see `engine_for`.
        (
            "bcu",
            vec!["win-uninstall", "Foo App"],
            vec!["win-uninstall", "--confidence", "Good"],
        ),
        // czkawka, and the same point in its sharpest form: a MISSING sidecar is still an
        // `ok:true` previewable command naming `czkawka`, so the failure has to name it too.
        ("czkawka", vec!["win-dupes", dir], vec!["win-dupes"]),
    ];
    // `slim` only has a success where there is a fat Mach-O for it to report on.
    if cfg!(target_os = "macos") {
        cases.push((
            "native",
            vec!["slim", "/bin/ls"],
            vec!["slim", "--output", dir],
        ));
    }

    for (expected, good, bad) in cases {
        let run = |args: &[&str]| {
            let out = burrow()
                .env("BURROW_SCAN_DIR", &scans)
                .env("BURROW_BCU", &absent)
                .env("BURROW_CZKAWKA", &absent)
                .args(args)
                .output()
                .unwrap();
            let s = String::from_utf8_lossy(&out.stdout).into_owned();
            let v: serde_json::Value = serde_json::from_str(s.trim())
                .unwrap_or_else(|e| panic!("{args:?}: one envelope ({e}): {s}"));
            (out.status.success(), v)
        };
        let (ok, success) = run(&good);
        let (failed, failure) = run(&bad);

        assert!(ok, "{good:?} must succeed: {success}");
        assert!(!failed, "{bad:?} must fail: {failure}");
        assert_eq!(
            failure["engine"], success["engine"],
            "{good:?} and {bad:?} are one command naming two engines: {success} / {failure}"
        );
        assert_eq!(
            success["engine"],
            serde_json::json!(expected),
            "got: {success}"
        );
    }

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&scans);
}

/// `--raw` must be consumed by the conductor on the moved commands too. The engine refuses the
/// flag with exit 2, so a conductor that forwarded it would turn every `--raw` call into a hard
/// failure — and these commands newly go through that path.
#[test]
fn raw_is_consumed_and_yields_the_bare_payload_on_moved_commands() {
    for cmd in ["photos", "net", "orphans"] {
        let (ok, s) = stdout(burrow().args([cmd, "/tmp", "--raw"]));
        assert!(ok, "{cmd} --raw must not reach the engine: {s}");
        assert!(
            !s.contains("burrow_cli"),
            "{cmd} --raw must be the bare payload: {s}"
        );
        assert!(!s.contains("--raw"), "{cmd} --raw leaked into argv: {s}");
    }
}

/// `evict` takes `--apply` and no `--dry-run`, so the only honest translation is present-or-
/// absent — and a `--dry-run` must never be invented for it, which the engine refuses outright.
#[test]
fn evict_apply_reaches_the_engine_and_never_gains_a_dry_run() {
    let (ok, s) = stdout(burrow().args(["evict", "/tmp/x", "--apply"]));
    assert!(ok, "got: {s}");
    assert!(
        s.contains(r#""--apply""#),
        "apply must reach the engine: {s}"
    );
    assert!(
        !s.contains(r#""--dry-run""#),
        "evict has no --dry-run to send: {s}"
    );
}

/// On a command that cannot write, `--apply` must reach the engine and be REFUSED there rather
/// than swallowed here — swallowing would report a write as honoured that nothing could perform.
#[test]
fn apply_on_a_read_only_moved_command_is_forwarded_not_swallowed() {
    let (_, s) = stdout(burrow().args(["photos", "/tmp", "--apply"]));
    assert!(s.contains(r#""--apply""#), "must be forwarded: {s}");
}

/// The one platform refusal the conductor still raises for a forwarded command, and its exact
/// scope.
///
/// `sentinel --watch` with no directory watches the INFERRED `<home>/.Trash`, which is a macOS
/// path — the daemon would report "nothing trashed" forever about a Recycle Bin it never opened.
/// The engine's own gate on the same inference cannot cover this one: `--watch` is the
/// conductor's poll loop and never reaches the engine at all, which is what makes this refusal
/// load-bearing rather than the leftover the two it replaced turned out to be.
///
/// This replaces `windows_refuses_the_commands_the_engine_would_answer_falsely`, which pinned the
/// `engine::windows_refusal` table retired (pre-squash `a87b4b1`, part of #18). That test asserted `sentinel` and `evict`
/// were refused BEFORE dispatch; the opposite is now the contract, and
/// `the_two_commands_that_used_to_be_pre_empted_now_reach_the_engine` asserts it on every
/// platform. Being `#[cfg(windows)]` it never compiled on the machine that change was made on,
/// which is how it outlived the behavior it described — the reason this file's Windows-only
/// coverage is kept to claims that stay true when the engine takes something over.
#[cfg(windows)]
#[test]
fn windows_refuses_only_the_trash_directory_it_would_have_had_to_invent() {
    let out = burrow().args(["sentinel", "--watch"]).output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "got: {s}");
    assert!(s.contains(r#""kind":"unsupported""#), "got: {s}");
    // README documents the top-level `feature` key as how a caller tells a conductor's own
    // refusal from a relayed engine one, and this is the conductor's own.
    assert!(s.contains(r#""feature":"sentinel --watch""#), "got: {s}");

    // ...and ONLY that. A directory the caller named is a `read_dir` and a `.app` suffix test,
    // which needs no platform vocabulary; refusing it would take a working command away, which is
    // the over-refusal the retired table was guilty of. One tick is the smallest valid bound;
    // watch a fresh empty fixture so this neither sleeps nor scans the runner's shared temp tree.
    let named =
        std::env::temp_dir().join(format!("burrow_windows_sentinel_{}", std::process::id()));
    std::fs::create_dir(&named).unwrap();
    let (ok, s2) = stdout(burrow().args([
        "sentinel",
        named.to_str().unwrap(),
        "--watch",
        "--max-ticks",
        "1",
    ]));
    assert!(ok, "a named directory must not be refused: {s2}");
    assert!(!s2.contains("unsupported"), "got: {s2}");

    // The refusal above still credits whoever SERVES `sentinel` — the engine, which runs its
    // one-shot scan. `engine` answers "who serves this command" on every envelope the command can
    // produce, and giving it a second meaning on a refusal is the split that
    // `a_commands_failure_names_the_same_engine_as_its_success` exists to prevent; the `feature`
    // key is what says the conductor refused, per README. Read off a real one-shot run rather
    // than named here, so the refusal cannot drift away from the dispatch it is refusing.
    let (scan_ok, scan) = stdout(burrow().args(["sentinel", named.to_str().unwrap()]));
    assert!(scan_ok, "the one-shot scan must reach the engine: {scan}");
    let dispatched: serde_json::Value = serde_json::from_str(scan.trim()).expect("one envelope");
    let refused: serde_json::Value = serde_json::from_str(s.trim()).expect("one envelope");
    assert_eq!(
        refused["engine"], dispatched["engine"],
        "one command naming two engines: {scan} / {s}"
    );
    std::fs::remove_dir(named).unwrap();
}

#[test]
fn sentinel_watches_an_explicit_directory_for_one_tick() {
    let root = std::env::temp_dir().join(format!("burrow_sentinel_tick_{}", std::process::id()));
    let app = root.join("Reviewed.app");
    std::fs::create_dir_all(&app).unwrap();
    let out = burrow()
        .args([
            "sentinel",
            root.to_str().unwrap(),
            "--watch",
            "--max-ticks",
            "1",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let event: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(event["event"], "trashed_app");
    assert_eq!(event["path"], app.to_string_lossy().as_ref());
    assert!(app.is_dir());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn trigger_rules_emits_plan_only_for_auto_rules_when_fired() {
    use std::fs;
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("burrow_trigrules_{pid}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let auto_target = dir.join("auto_cache");
    let manual_target = dir.join("manual_cache");
    fs::write(&auto_target, "x").unwrap();
    fs::write(&manual_target, "x").unwrap();
    let rf = serde_json::json!({
        "schema": "burrow.rules/v1",
        "app": { "bundle_ids": ["com.x.y"], "name": "X" },
        "rules": [
            {"id": "x.auto", "category": "cache", "risk": "safe", "recommend": true,
             "explain": "e", "auto": true,
             "targets": [{ "path": auto_target.to_string_lossy() }],
             "action": { "type": "delete", "method": "trash" }},
            {"id": "x.manual", "category": "cache", "risk": "safe", "recommend": true,
             "explain": "e",
             "targets": [{ "path": manual_target.to_string_lossy() }],
             "action": { "type": "delete", "method": "trash" }}
        ],
        "provenance": { "source": "builtin" }
    });
    fs::write(dir.join("com.x.y.json"), rf.to_string()).unwrap();

    // Threshold 0 -> always fires: the plan lists ONLY the auto-opted rule's target,
    // in trash --apply-plan's bare-array shape (compose: write plan -> apply).
    let (ok, s) = stdout(burrow().args([
        "trigger",
        "--threshold",
        "0",
        "--rules",
        dir.to_str().unwrap(),
    ]));
    assert!(ok, "got: {s}");
    assert!(s.contains(r#""should_clean":true"#), "got: {s}");
    assert!(s.contains("auto_cache"), "auto rule must be planned: {s}");
    assert!(
        !s.contains("manual_cache"),
        "non-auto rules must NEVER be auto-planned: {s}"
    );

    // Not fired -> empty plan (nothing is ever planned below threshold).
    let (_, s2) = stdout(burrow().args([
        "trigger",
        "--threshold",
        "100",
        "--rules",
        dir.to_str().unwrap(),
    ]));
    assert!(s2.contains(r#""should_clean":false"#), "got: {s2}");
    assert!(!s2.contains("auto_cache"), "no plan when not fired: {s2}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn report_weekly_is_a_digest_alias() {
    // Plan Phase 8 names this `burrow report --weekly`; it aliases the digest surface.
    let (ok, s) = stdout(burrow().args(["report", "--weekly"]));
    assert!(ok, "got: {s}");
    assert!(s.contains(r#""command":"digest""#), "got: {s}");
}

#[cfg(target_os = "macos")]
#[test]
fn gui_missing_app_emits_error_envelope() {
    // "Every command emits the envelope" — the gui not-installed miss must be a classified
    // error envelope on stdout, not bare stderr text. BURROW_GUI_APP pins the app location
    // so the miss is deterministic (and nothing is ever launched from a test).
    let out = burrow()
        .env("BURROW_GUI_APP", "/nonexistent/Burrow.app")
        .arg("gui")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains(r#""ok":false"#), "got: {s}");
    assert!(s.contains(r#""command":"gui""#), "got: {s}");
    assert!(s.contains(r#""kind":"not_found""#), "got: {s}");
}

// --- dupes (fclones sidecar) ---

// --- Windows duplicate discovery (czkawka) ---

#[test]
fn win_dupes_normalizes_czkawka_exit_11_report() {
    let (root, executable) = fake_czkawka();
    let (ok, s) = stdout(
        burrow()
            .env("BURROW_CZKAWKA", &executable)
            .args(["win-dupes", "C:/scan"]),
    );
    assert!(ok, "czkawka exit 11 means items found: {s}");
    assert!(s.contains(r#""engine":"czkawka""#), "got: {s}");
    assert!(s.contains(r#""scan":"duplicates""#), "got: {s}");
    assert!(s.contains(r#""group_count":1"#), "got: {s}");
    assert!(s.contains(r#""file_count":2"#), "got: {s}");
    assert!(s.contains(r#""redundant_bytes":64"#), "got: {s}");
    assert!(
        !s.contains(r#""report""#),
        "raw report must be omitted: {s}"
    );
    // `dupes` used to fall back to this same czkawka driver on Windows when no fclones was
    // present, and that fallback was asserted here. `dupes` is the engine's now on every
    // platform, so the czkawka surface is reachable only as `win-dupes` — which is what keeps
    // this driver, and this test, alive.
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn missing_czkawka_emits_a_previewable_command() {
    let missing =
        std::env::temp_dir().join(format!("burrow_missing_czkawka_{}", std::process::id()));
    let _ = std::fs::remove_file(&missing);
    let (ok, s) = stdout(
        burrow()
            .env("BURROW_CZKAWKA", &missing)
            .args(["win-dupes", "C:/scan"]),
    );
    assert!(ok, "missing sidecar should return a discovery preview: {s}");
    assert!(s.contains(r#""available":false"#), "got: {s}");
    assert!(s.contains(r#""engine":"czkawka""#), "got: {s}");
    assert!(s.contains(r#""scan":"duplicates""#), "got: {s}");
    assert!(s.contains(r#""would_run":"czkawka_cli dup"#), "got: {s}");
    assert!(s.contains(r#""args":["dup","-d","C:/scan""#), "got: {s}");
    assert!(s.contains(r#""reason":"$BURROW_CZKAWKA"#), "got: {s}");
    assert!(
        !s.contains(r#""groups""#),
        "preview is not an empty scan: {s}"
    );
}

#[test]
fn malformed_czkawka_json_is_an_invalid_output_failure() {
    let (root, executable) = fake_executable(
        "czkawka_cli_malformed",
        "@echo off\r\nset \"out=\"\r\n:loop\r\nif \"%~1\"==\"\" goto done\r\nset \"out=%~1\"\r\nshift\r\ngoto loop\r\n:done\r\n> \"%out%\" echo not-json\r\nexit /b 0\r\n",
        "#!/bin/sh\nout=\"\"\nfor arg in \"$@\"; do out=\"$arg\"; done\nprintf '%s\\n' 'not-json' > \"$out\"\nexit 0\n",
    );
    let out = burrow()
        .env("BURROW_CZKAWKA", &executable)
        .args(["win-dupes", "C:/scan"])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "got: {s}");
    assert!(s.contains(r#""kind":"invalid_output""#), "got: {s}");
    let _ = std::fs::remove_dir_all(root);
}

/// `--apply` names a write czkawka discovery cannot perform anywhere, so it is refused on EVERY
/// platform. The refusal used to be gated on `cfg!(windows)`; on macOS the same invocation was
/// folded into an `ok:true` preview — a caller who asked for a mutation told it had succeeded.
/// A swallowed `--apply` is the one thing this conductor promises never to do.
#[test]
fn win_dupes_apply_is_refused_everywhere_never_folded_into_a_preview() {
    let out = burrow()
        .args(["win-dupes", "C:/scan", "--apply"])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "got: {s}");
    let v: serde_json::Value = serde_json::from_str(s.trim()).expect("one envelope");
    assert_eq!(v["ok"], serde_json::json!(false), "got: {s}");
    assert_eq!(
        v["error"]["kind"],
        serde_json::json!("unsupported"),
        "got: {s}"
    );
    // The conductor's own refusal, marked as such — see README on the `feature` key.
    assert_eq!(
        v["feature"],
        serde_json::json!("win-dupes --apply"),
        "got: {s}"
    );
    assert!(
        v.get("data").is_none(),
        "a refused write must not carry a preview: {s}"
    );
}

// --- Windows uninstall (BCU) ---

#[test]
fn win_uninstall_preview_never_resolves_or_runs_bcu() {
    let missing = std::env::temp_dir().join(format!("burrow_missing_bcu_{}", std::process::id()));
    let _ = std::fs::remove_file(&missing);
    let (ok, s) = stdout(
        burrow()
            .env("BURROW_BCU", &missing)
            .args(["win-uninstall", "Foo App"]),
    );
    assert!(ok, "preview must not resolve the missing executable: {s}");
    assert!(s.contains(r#""engine":"bcu""#), "got: {s}");
    assert!(s.contains(r#""applied":false"#), "got: {s}");
    // The preview names the $BURROW_BCU override VERBATIM (the program that would run) —
    // still without resolving or stat-ing it; a hardcoded "BCU-console.exe" would
    // contradict the override.
    let program = missing.to_string_lossy().replace('\\', "\\\\");
    assert!(s.contains(&format!(r#""program":"{program}""#)), "got: {s}");
    assert!(s.contains(r#""args":["list","Foo App"]"#), "got: {s}");
    assert!(!s.contains("fake BCU success"), "BCU must not run: {s}");
}

#[test]
fn win_uninstall_apply_reports_success_and_exact_args() {
    let (root, executable) = fake_bcu("", 0);
    let (ok, s) = stdout(burrow().env("BURROW_BCU", &executable).args([
        "win-uninstall",
        "Foo App",
        "--confidence",
        "Good",
        "--apply",
    ]));
    assert!(ok, "got: {s}");
    assert!(s.contains(r#""engine":"bcu""#), "got: {s}");
    assert!(s.contains(r#""applied":true"#), "got: {s}");
    assert!(
        s.contains(r#""args":["uninstall","Foo App","/Q","/U","/J=Good"]"#),
        "got: {s}"
    );
    assert!(s.contains(r#""exit_code":0"#), "got: {s}");
    assert!(s.contains("fake BCU success"), "got: {s}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn missing_bcu_apply_is_a_structured_failure() {
    let missing =
        std::env::temp_dir().join(format!("burrow_missing_bcu_apply_{}", std::process::id()));
    let _ = std::fs::remove_file(&missing);
    let out = burrow()
        .env("BURROW_BCU", &missing)
        .args(["win-uninstall", "Foo App", "--apply"])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "got: {s}");
    assert!(s.contains(r#""kind":"not_found""#), "got: {s}");
    assert!(s.contains(r#""details":{"applied":true"#), "got: {s}");
    assert!(
        s.contains(r#""args":["uninstall","Foo App","/Q","/U","/J=Good"]"#),
        "got: {s}"
    );
}

/// ...and that envelope must not contradict itself.
///
/// This is the same failure as above, and it is the one that made the hardcoded attribution
/// impossible to defend: the `error.details` this call site attaches already say
/// `"engine":"bcu"` — the engine it tried and could not find — while the envelope wrapped around
/// them said `"engine":"burrow-engine"`. One document, two answers to the same question.
///
/// Both values are read back OUT of the emitted envelope, so this names no engine of its own and
/// keeps holding if the attribution is ever deliberately changed to something else.
#[test]
fn a_failures_engine_agrees_with_the_details_it_attaches() {
    let missing = std::env::temp_dir().join(format!("burrow_bcu_agree_{}", std::process::id()));
    let _ = std::fs::remove_file(&missing);
    let out = burrow()
        .env("BURROW_BCU", &missing)
        .args(["win-uninstall", "Foo App", "--apply"])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(s.trim()).unwrap_or_else(|e| panic!("one envelope ({e}): {s}"));

    assert!(!out.status.success(), "got: {s}");
    assert!(
        v["error"]["details"]["engine"].is_string(),
        "the details must actually name an engine or this proves nothing: {s}"
    );
    assert_eq!(
        v["engine"], v["error"]["details"]["engine"],
        "the envelope contradicts the details it carries: {s}"
    );
}

#[test]
fn bcu_generic_nonzero_is_a_process_failure() {
    let (root, executable) = fake_bcu("installer crashed", 7);
    let out = burrow()
        .env("BURROW_BCU", &executable)
        .args(["win-uninstall", "Foo App", "--apply"])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "got: {s}");
    assert!(s.contains(r#""kind":"process_failed""#), "got: {s}");
    assert!(s.contains(r#""exit_code":7"#), "got: {s}");
    assert!(s.contains(r#""stderr":"installer crashed""#), "got: {s}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn bcu_permission_and_uac_phrases_are_permission_failures() {
    for phrase in [
        "Access is denied",
        "The requested operation requires elevation",
        "permission denied",
    ] {
        let (root, executable) = fake_bcu(phrase, 5);
        let out = burrow()
            .env("BURROW_BCU", &executable)
            .args(["win-uninstall", "Foo App", "--apply"])
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(!out.status.success(), "phrase={phrase}, got: {s}");
        assert!(
            s.contains(r#""kind":"permission_denied""#),
            "phrase={phrase}, got: {s}"
        );
        assert!(s.contains(phrase), "phrase={phrase}, got: {s}");
        let _ = std::fs::remove_dir_all(root);
    }
}

// --- rules engine (validate the shipped seed rules) ---

// --- orphan scanner (filesystem) ---

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

    // Without `--apply` the plan is PREVIEWED: the same `would_trash` shape the original
    // preview had, and the file untouched. `--apply-plan` used to delete on sight.
    let out = burrow()
        .args(["trash", "--apply-plan", plan.to_str().unwrap()])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "got: {s}");
    assert!(s.contains(r#""applied":false"#), "got: {s}");
    assert!(s.contains(r#""would_trash""#), "got: {s}");
    assert!(
        victim.exists(),
        "--apply-plan without --apply must not delete: {s}"
    );

    let out = burrow()
        .args(["trash", "--apply-plan", plan.to_str().unwrap(), "--apply"])
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
        .args([
            "trash",
            "--apply-plan",
            plan.to_str().unwrap(),
            "--apply",
            "--stream",
        ])
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

#[test]
fn native_arguments_are_validated_before_any_action() {
    let (root, bcu) = fake_bcu("", 0);
    let victim = root.join("keep.txt");
    std::fs::write(&victim, "keep").unwrap();
    let path = victim.to_str().unwrap();
    for args in [
        vec!["trash", path, "--apply", "--dry-run"],
        vec!["trash", path, "--apply", "--dryrun"],
        vec!["trash", path, "--apply", "--apply-plan"],
        vec!["win-uninstall", "Foo", "--apply", "--dry-run"],
        vec!["win-uninstall", "Foo", "--apply", "--confidence"],
        vec!["win-uninstall", "Foo", "Extra", "--apply"],
        vec!["sentinel", root.to_str().unwrap(), "--watch", "--apply"],
        vec![
            "sentinel",
            root.to_str().unwrap(),
            "--watch",
            "--max-ticks",
            "nonsense",
        ],
        vec![
            "sentinel",
            root.to_str().unwrap(),
            "--watch",
            "--interval-ms",
            "0",
        ],
        vec!["trigger", "--threshold", "NaN"],
        vec!["trigger", "--threshold", "101"],
        vec!["digest", "--days", "9223372036854775807"],
        vec!["digest", "--days", "invalid"],
        vec!["watch", "--apply"],
    ] {
        let out = burrow()
            .env("BURROW_BCU", &bcu)
            .args(&args)
            .output()
            .unwrap();
        assert!(!out.status.success(), "{args:?}: {:?}", out);
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value["ok"], false, "{args:?}");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn trash_reuses_its_complete_preview_and_refuses_a_malformed_plan() {
    let (root, _) = fake_bcu("", 0);
    let victim = root.join("keep.txt");
    std::fs::write(&victim, "keep").unwrap();
    let (_, preview) = stdout(burrow().args(["trash", victim.to_str().unwrap()]));
    let plan = root.join("plan.json");
    std::fs::write(&plan, &preview).unwrap();
    let (ok, replay) = stdout(burrow().args(["trash", "--apply-plan", plan.to_str().unwrap()]));
    assert!(ok, "{replay}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&preview).unwrap(),
        serde_json::from_str::<serde_json::Value>(&replay).unwrap()
    );
    std::fs::write(
        &plan,
        serde_json::json!([victim, {"path": "wrong shape"}]).to_string(),
    )
    .unwrap();
    let (ok, result) =
        stdout(burrow().args(["trash", "--apply-plan", plan.to_str().unwrap(), "--apply"]));
    assert!(!ok, "{result}");
    assert!(
        victim.exists(),
        "a malformed plan must be rejected before its first deletion"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_trash_targets_fail_both_buffered_and_streaming_calls() {
    let (root, _) = fake_bcu("", 0);
    let missing = root.join("absent");
    for stream in [false, true] {
        let mut command = burrow();
        command.args(["trash", missing.to_str().unwrap(), "--apply"]);
        if stream {
            command.arg("--stream");
        }
        let (ok, output) = stdout(&mut command);
        assert!(!ok, "{output}");
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["ok"], false);
        if !stream {
            assert_eq!(value["error"]["details"]["trashed"][0]["ok"], false);
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_scan_roots_do_not_become_successful_empty_scans() {
    let (root, _) = fake_bcu("", 0);
    let missing = root.join("absent");
    for args in [
        vec!["diff", missing.to_str().unwrap()],
        vec![
            "sentinel",
            missing.to_str().unwrap(),
            "--watch",
            "--max-ticks",
            "1",
        ],
    ] {
        let (ok, output) = stdout(
            burrow()
                .env("BURROW_SCAN_DIR", root.join("scans"))
                .args(args),
        );
        assert!(!ok, "{output}");
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["error"]["kind"], "not_found");
    }
    assert!(!root.join("scans").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_raw_and_unknown_command_respect_the_output_contract() {
    let (_, output) = stdout(burrow().args(["trash", "/missing", "--raw"]));
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert!(value.get("would_trash").is_some());
    assert!(value.get("data").is_none());
    let (ok, output) = stdout(burrow().arg("unknown\"command"));
    assert!(!ok);
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["command"], "unknown\"command");
}

#[test]
fn engine_failure_envelopes_and_details_survive_a_zero_exit() {
    let report = r#"{"ok":false,"error":{"kind":"permission_denied","message":"cannot remove target","details":{"failed":1,"path":"/fixture"}},"data":{"misleading":true}}"#;
    let (root, binary) = fake_executable(
        "failed_engine",
        &format!("@echo off\r\necho {report}\r\nexit /b 0\r\n"),
        &format!("#!/bin/sh\nprintf '%s\\n' '{report}'\nexit 0\n"),
    );
    let (ok, output) = stdout(burrow().env("BURROW_ENGINE", &binary).arg("clean"));
    assert!(!ok, "{output}");
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["error"]["kind"], "permission_denied");
    assert_eq!(value["error"]["details"]["failed"], 1);
    assert_eq!(value["error"]["details"]["path"], "/fixture");
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn slim_never_overwrites_an_existing_file_or_input_alias() {
    let (root, _) = fake_bcu("", 0);
    let input = root.join("fat.bin");
    let output = root.join("existing.bin");
    let mut fat = Vec::new();
    for word in [
        0xcafebabe_u32,
        1,
        if cfg!(target_arch = "aarch64") {
            0x0100000c
        } else {
            0x01000007
        },
        0,
        28,
        4,
        0,
    ] {
        fat.extend_from_slice(&word.to_be_bytes());
    }
    fat.extend_from_slice(b"body");
    std::fs::write(&input, &fat).unwrap();
    std::fs::write(&output, b"existing").unwrap();
    let link = root.join("alias");
    std::os::unix::fs::symlink(&input, &link).unwrap();
    let hardlink = root.join("hardlink");
    std::fs::hard_link(&input, &hardlink).unwrap();
    for destination in [&input, &output, &link, &hardlink] {
        let (ok, output) = stdout(burrow().args([
            "slim",
            input.to_str().unwrap(),
            "--apply",
            "--output",
            destination.to_str().unwrap(),
        ]));
        assert!(!ok, "{output}");
        assert_eq!(std::fs::read(&input).unwrap(), fat);
    }
    assert_eq!(std::fs::read(output).unwrap(), b"existing");
    // A slice that cannot be signed must not be reported as a successful runnable output.
    let unsigned = root.join("unsigned");
    let (signer_root, _) = fake_executable("codesign", "", "#!/bin/sh\necho denied >&2\nexit 1\n");
    let (ok, output) = stdout(burrow().env("PATH", &signer_root).args([
        "slim",
        input.to_str().unwrap(),
        "--apply",
        "--output",
        unsigned.to_str().unwrap(),
    ]));
    assert!(!ok, "{output}");
    assert!(!unsigned.exists());
    std::fs::remove_dir_all(signer_root).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
