//! Burrow CLI — agent-native system-cleaning conductor.
//!
//! Dispatches every command per the table in `commands.rs`: to the `burrow-engine` binary
//! (`engine.rs`), to the czkawka/BCU sidecars behind the two `win-` commands, or to this crate's
//! own handlers below. Every result is wrapped in a stable Burrow envelope. Destructive commands
//! default to dry-run/preview; `--apply` executes (explain-before-delete).

mod bcu;
mod commands;
mod czkawka;
mod engine;
mod gui;
mod macho;
mod metrics;
mod output;
mod platform;
mod recycle;
mod rules;
mod sentinel;
mod telemetry;
mod treediff;

use std::process::ExitCode;

// Tracks whether the command failed (set by `fail`), for telemetry's success field.
thread_local! {
    static OK: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    static RAW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args
        .first()
        .map(String::as_str)
        .unwrap_or("help")
        .to_string();

    // `telemetry` is handled here directly and is never itself recorded.
    if cmd == "telemetry" {
        println!("{}", telemetry::command(&args[1..]));
        return ExitCode::SUCCESS;
    }

    telemetry::maybe_first_run_notice();
    let start = std::time::Instant::now();
    let code = run(&cmd, &args);
    // Record real commands only — the command NAME (never args), success, coarse duration.
    if commands::spec(&cmd).is_some()
        && !matches!(
            cmd.as_str(),
            "help" | "--help" | "-h" | "version" | "--version" | "-V"
        )
    {
        telemetry::record(
            &cmd,
            OK.with(|o| o.get()),
            start.elapsed().as_millis() as u64,
        );
    }
    code
}

fn run(cmd: &str, args: &[String]) -> ExitCode {
    match cmd {
        "version" | "--version" | "-V" => {
            println!(
                "burrow {} (conductor; engine: burrow-engine)",
                env!("CARGO_PKG_VERSION")
            );
            return ExitCode::SUCCESS;
        }
        "help" | "--help" | "-h" => {
            print_help();
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    let Some(spec) = commands::spec(cmd) else {
        return fail(cmd, format!("unknown command '{cmd}'; run `burrow help`"));
    };
    RAW.with(|raw| raw.set(args.iter().any(|arg| arg == "--raw")));
    // Dispatch is the table's decision, not a match here that has to agree with it. A command
    // with a native handler runs it (an alias runs its canonical row's); everything else the
    // table gave the engine is forwarded; a row with neither home is a table bug, pinned by
    // `every_command_has_exactly_one_home`, and says so rather than silently doing nothing.
    match native_handler(spec.name) {
        Some(handler) => {
            if spec.name != "sentinel" || args.iter().any(|arg| arg == "--watch") {
                if let Err(error) = validate_native_args(spec.name, &args[1..]) {
                    return fail(spec.name, error);
                }
            }
            handler(&args[1..])
        }
        None if engine::owns(spec.name) => run_engine(spec.name, &args[1..]),
        None => fail(
            spec.name,
            format!("{} has no handler in this build", spec.name),
        ),
    }
}

/// Validate native arguments before a handler can read a file, spawn a sidecar, or mutate.
/// Unknown flags must not disappear: a misspelled preview or an incomplete plan flag can
/// otherwise become a successful write against the remaining positional paths.
fn validate_native_args(cmd: &str, args: &[String]) -> Result<(), String> {
    let (flags, value_flags, min_paths, max_paths): (&[&str], &[&str], usize, usize) = match cmd {
        "diff" => (&[], &[], 1, 1),
        "snapshot" | "watch" => (&[], &[], 0, 0),
        "digest" => (&["--weekly"], &["--days"], 0, 0),
        "trigger" => (&[], &["--threshold", "--rules"], 0, 0),
        "slim" => (&["--apply", "--dry-run"], &["--output"], 1, 1),
        "gui" => (&["--install"], &[], 0, 0),
        "trash" => (
            &["--apply", "--dry-run", "--stream"],
            &["--apply-plan"],
            0,
            usize::MAX,
        ),
        "win-uninstall" => (&["--apply", "--dry-run"], &["--confidence"], 1, 1),
        // Keep the explicit unsupported classification in run_win_dupes.
        "win-dupes" => (&["--images", "--apply", "--dry-run"], &[], 1, 1),
        "sentinel" => (&["--watch"], &["--interval-ms", "--max-ticks"], 0, 1),
        _ => return Err(format!("no argument schema for {cmd}")),
    };
    if args.iter().any(|arg| arg == "--apply") && args.iter().any(|arg| arg == "--dry-run") {
        return Err("--apply and --dry-run cannot be combined".into());
    }
    let mut paths = 0;
    let mut seen = std::collections::HashSet::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if !arg.starts_with('-') {
            paths += 1;
            continue;
        }
        if !seen.insert(arg.as_str()) {
            return Err(format!("duplicate option: {arg}"));
        }
        if value_flags.contains(&arg.as_str()) {
            let value = iter
                .next()
                .filter(|value| !value.starts_with('-') && !value.is_empty())
                .ok_or_else(|| format!("{arg} needs a value"))?;
            match arg.as_str() {
                "--days" => {
                    let days = value
                        .parse::<i64>()
                        .ok()
                        .filter(|days| *days > 0)
                        .and_then(|days| days.checked_mul(86_400));
                    if days.is_none() {
                        return Err("--days needs a positive number of days within range".into());
                    }
                }
                "--interval-ms" | "--max-ticks"
                    if !value.parse::<u64>().is_ok_and(|value| value > 0) =>
                {
                    return Err(format!("{arg} needs a positive integer"));
                }
                "--threshold"
                    if !value
                        .parse::<f64>()
                        .is_ok_and(|value| value.is_finite() && (0.0..=100.0).contains(&value)) =>
                {
                    return Err("--threshold needs a percentage from 0 to 100".into());
                }
                _ => {}
            }
        } else if !flags.contains(&arg.as_str()) && !["--json", "--raw"].contains(&arg.as_str()) {
            return Err(format!("unknown {cmd} option: {arg}"));
        }
    }
    if paths < min_paths {
        return Err(match cmd {
            "slim" => "needs a path to a Mach-O binary",
            "win-uninstall" => "needs an app name",
            _ => "needs a directory to scan",
        }
        .into());
    }
    if paths > max_paths {
        return Err(format!(
            "{cmd}: unexpected number of positional arguments; run `burrow help`"
        ));
    }
    if cmd == "trash" {
        let has_plan = seen.contains("--apply-plan");
        if has_plan && paths > 0 {
            return Err("trash: use either --apply-plan or positional paths".into());
        }
        if !has_plan && paths == 0 {
            return Err("trash: needs at least one path or --apply-plan".into());
        }
        if seen.contains("--stream") && !seen.contains("--apply") {
            return Err("trash: --stream requires --apply".into());
        }
    }
    Ok(())
}

/// The conductor's own handlers, by canonical command name.
///
/// Every table row the engine does not serve has one. `sentinel` has one too although the table
/// gives it to the engine, because its `--watch` half is the conductor's poll loop — `run_sentinel`
/// forwards the one-shot scan and keeps the daemon.
fn native_handler(name: &str) -> Option<fn(&[String]) -> ExitCode> {
    Some(match name {
        "diff" => run_diff,
        "snapshot" => run_snapshot,
        "digest" => run_digest,
        "watch" => run_watch,
        "trigger" => run_trigger,
        "slim" => run_slim,
        "gui" => run_gui,
        "trash" => run_trash,
        "win-uninstall" => run_win_uninstall,
        "win-dupes" => run_win_dupes,
        "sentinel" => run_sentinel,
        _ => return None,
    })
}

/// Resolve the engine and run the command.
///
/// Two transports, and the choice no longer depends on whether stdout is a terminal. It used to:
/// the digger rendered a colored TUI worth showing a human, so an interactive run inherited
/// stdio and a piped one got the envelope. `burrow-engine` has exactly one output mode, so
/// inheriting stdio buys nothing and costs two things — the ENGINE's envelope reaches stdout
/// (carrying the engine's version in `burrow_cli`, which names the conductor) and `--raw` is
/// bypassed. So everything is captured and re-emitted through the conductor's own envelope,
/// EXCEPT a genuine stream — `clean`/`optimize`/`purge --stream`, `status --watch`,
/// `analyze --progress` — which has no envelope to wrap: it is NDJSON, one line per frame, and
/// must reach the caller as it happens. The engine's exit code is the stream's, relayed as-is.
fn run_engine(cmd: &str, args: &[String]) -> ExitCode {
    let raw = args.iter().any(|a| a == "--raw");

    let plan = match engine::plan(cmd, args) {
        Ok(p) => p,
        Err(e) => return fail(cmd, e),
    };
    let bin = match engine::resolve() {
        Ok(b) => b,
        Err(e) => return fail(cmd, e),
    };

    if engine::is_stream_passthrough(cmd, args) {
        return match engine::execute_native(&bin, &plan) {
            Ok(0) => ExitCode::SUCCESS,
            Ok(code) => {
                OK.with(|ok| ok.set(false));
                ExitCode::from(nonzero_exit_byte(code))
            }
            Err(e) => fail(cmd, e),
        };
    }
    match engine::execute(&bin, &plan) {
        Ok(out) => emit(cmd, &out, raw),
        Err(e) => fail(cmd, e),
    }
}

/// Narrow a NONZERO engine exit code to the byte an [`ExitCode`] can carry, without ever
/// reporting a failure as a success.
///
/// `ExitCode::from(code as u8)` is the obvious spelling and it is wrong: the cast keeps only the
/// low byte, so any code whose low byte is zero — 256, 0x10000 — becomes `ExitCode::from(0)`,
/// which IS success. On Unix that is latent, because the wait status is already masked to 8 bits
/// before `.code()` ever sees it; on Windows an exit code is a full 32-bit value and a real
/// failure would be relayed as a clean run. The conductor is one binary, so it takes the reading
/// that holds on both.
///
/// A code that does not fit in a byte is reported as 1. The exact number is unrecoverable either
/// way — `ExitCode` carries a byte and nothing else — but "it failed" is not, and that is the
/// part a caller branches on.
fn nonzero_exit_byte(code: i32) -> u8 {
    match u8::try_from(code) {
        // Unreachable for a nonzero `code`, but stated so the guarantee is local to this
        // function rather than resting on the caller's match arms.
        Ok(0) | Err(_) => 1,
        Ok(byte) => byte,
    }
}

/// `burrow diff <dir>` — disk-growth diff vs the previous scan of the same dir (first run
/// saves a baseline). Read-only except for the saved scan file.
fn run_diff(args: &[String]) -> ExitCode {
    let root = match args.iter().find(|a| !a.starts_with("--")) {
        Some(r) => r.clone(),
        None => return fail("diff", "needs a directory to scan"),
    };
    let curr = match treediff::scan_tree(std::path::Path::new(&root), 4) {
        Ok(scan) => scan,
        Err(error) => return fail("diff", error),
    };
    let v = match treediff::latest_scan(&root) {
        Some((ts, prev)) => {
            let mut changes = treediff::diff(&prev, &curr);
            changes.truncate(50);
            serde_json::json!({ "root": root, "baseline_ts": ts, "dirs_now": curr.len(), "changes": changes })
        }
        None => serde_json::json!({ "root": root, "baseline_saved": true, "dirs": curr.len() }),
    };
    if let Err(e) = treediff::save_scan(&root, &curr) {
        return fail("diff", e);
    }
    emit("diff", &v.to_string(), false)
}

/// `burrow slim <binary> [--apply --output <path>]` — thin a fat Mach-O to the host arch and
/// ad-hoc re-sign. Default reports; `--apply` writes (requires `--output`, never in-place).
fn run_slim(args: &[String]) -> ExitCode {
    // `--output` takes a value, so `slim --apply --output thin.bin /bin/ls` used to thin
    // `thin.bin` — the file it was told to WRITE — instead of the binary named after it.
    let path = match positionals(args, &["--output"]).into_iter().next() {
        Some(p) => p,
        None => return fail("slim", "needs a path to a Mach-O binary"),
    };
    if !platform::is_macos() {
        return emit_failure(
            "slim",
            "slim",
            "Mach-O thinning and ad-hoc codesign are macOS-only; Windows slimming is intentionally disabled.",
        );
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            return fail(
                "slim",
                output::Failure::io(format!("cannot read {path}"), &e),
            )
        }
    };
    let slices = match macho::parse_fat(&bytes) {
        Ok(s) => s,
        Err(e) => return fail("slim", e),
    };
    let keep = macho::host_cputype();

    if !args.iter().any(|a| a == "--apply") {
        let v = serde_json::json!({
            "input": path, "arch_count": slices.len(), "host_keep_cputype": keep,
            "potential_savings_bytes": macho::slim_savings(&slices, keep),
            "slices": slices, "applied": false,
        });
        return emit("slim", &v.to_string(), false);
    }

    let output = match flag_value(args, "--output") {
        Some(o) => o,
        None => {
            return fail(
                "slim",
                "--apply needs --output <path> (in-place thinning is not offered)",
            )
        }
    };
    let thinned = match macho::thin(&bytes, keep) {
        Ok(t) => t,
        Err(e) => return fail("slim", e),
    };
    use std::io::Write;
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
    {
        Ok(file) => file,
        Err(e) => {
            return fail(
                "slim",
                output::Failure::io(
                    format!("create {output}; --output must name a new file"),
                    &e,
                ),
            )
        }
    };
    let write_result = file
        .write_all(&thinned)
        .and_then(|()| std::fs::metadata(&path))
        .and_then(|metadata| file.set_permissions(metadata.permissions()));
    drop(file);
    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&output);
        return fail("slim", output::Failure::io(format!("write {output}"), &e));
    }
    let signed = std::process::Command::new("codesign")
        .args(["--force", "--sign", "-", &output])
        .output();
    match signed {
        Ok(result) if result.status.success() => {}
        result => {
            let _ = std::fs::remove_file(&output);
            let error = match result {
                Ok(result) => output::Failure::process_failed(format!(
                    "codesign failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                )),
                Err(error) => output::Failure::io("cannot run codesign", &error),
            };
            return fail("slim", error);
        }
    }
    let v = serde_json::json!({
        "input": path, "output": output,
        "original_size": bytes.len(), "thinned_size": thinned.len(),
        "reclaimed_bytes": bytes.len().saturating_sub(thinned.len()),
        "resigned_adhoc": true, "applied": true,
    });
    emit("slim", &v.to_string(), false)
}

/// `burrow sentinel [trashdir] --watch` — the poll-based Trash daemon.
///
/// The one-shot scan is the engine's, like every other overlapping command. `--watch` is NOT:
/// the engine has no watch mode and refuses the flag rather than answering a caller waiting on a
/// stream with a single snapshot, so the daemon stays here on every platform. It is also the mode
/// with a shipped consumer — the launchd templates in `packaging/launchd/` run it — so forwarding
/// it would break an integration, not just a flag.
fn run_sentinel(args: &[String]) -> ExitCode {
    /// `sentinel --watch`'s value-taking flags. The token after each belongs to the flag, so it
    /// is not the positional `[trashdir]` — see [`positionals`].
    const SENTINEL_VALUE_FLAGS: &[&str] = &["--interval-ms", "--max-ticks"];

    if !args.iter().any(|a| a == "--watch") {
        return run_engine("sentinel", args);
    }
    // ONE determination of what the caller named, feeding both the directory and the refusal
    // below. They used to be derived separately — `find(|a| !a.starts_with("--"))` for the
    // directory, `all(|a| a.starts_with("--"))` for the refusal — and both readings were wrong in
    // the same way and could disagree: `--interval-ms 500` made `500` the "directory" AND made the
    // refusal's "the caller named nothing" test false, so a numeric flag value both misdirected
    // the watch and bypassed the guard on the inference.
    let named = positionals(args, SENTINEL_VALUE_FLAGS);
    let inferred = named.is_empty();
    let dir = named
        .into_iter()
        .next()
        .unwrap_or_else(|| format!("{}/.Trash", platform::home_dir_string()));
    // No explicit directory on Windows means the default `~/.Trash`, which does not exist there.
    // Watching it would report "nothing trashed" forever about a Recycle Bin it never looked at.
    //
    // This is the ONE platform refusal left on the conductor's side of `sentinel`, and it is not a
    // leftover: `--watch` never reaches the engine at all, so the engine's own gate on the same
    // inference (`sentinel::default_trash_refusal`, added in `burrow-engine 5e42bf0`, which
    // retired the pre-dispatch refusal for the one-shot scan above) cannot cover it. The scope is
    // deliberately identical to the engine's — the INFERRED path only, never a directory the
    // caller named — so the two halves of `sentinel` refuse the same input and no other.
    if platform::is_windows() && inferred {
        return emit_failure(
            "sentinel",
            "sentinel --watch",
            "Trash sentinel watches macOS .app bundles; Windows Recycle Bin monitoring is not implemented.",
        );
    }
    // Poll-based: one NDJSON event per newly-trashed .app (a raw stream, no envelope — like
    // `status --watch`). The first tick announces the CURRENT trash contents, so a login item
    // starting up surfaces what is already actionable. `--max-ticks` bounds the loop for tests
    // and cron; the launchd templates run it unbounded. FSEvents is the native upgrade path.
    let interval = std::time::Duration::from_millis(
        flag_value(args, "--interval-ms")
            .and_then(|s| s.parse().ok())
            .unwrap_or(5_000),
    );
    let max_ticks: u64 = flag_value(args, "--max-ticks")
        .and_then(|s| s.parse().ok())
        .unwrap_or(u64::MAX);
    let mut prev: Vec<sentinel::TrashedApp> = Vec::new();
    let mut tick = 0u64;
    while tick < max_ticks {
        let curr = match sentinel::scan_trash(std::path::Path::new(&dir)) {
            Ok(apps) => apps,
            Err(error) => return fail("sentinel", error),
        };
        for app in sentinel::new_arrivals(&prev, &curr) {
            println!(
                "{}",
                serde_json::json!({
                    "event": "trashed_app",
                    "name": app.name,
                    "path": app.path,
                })
            );
        }
        prev = curr;
        tick += 1;
        if tick < max_ticks {
            std::thread::sleep(interval);
        }
    }
    ExitCode::SUCCESS
}

/// `burrow win-dupes <dir> [--images]` — duplicates / similar photos via czkawka_cli, read-only
/// (czkawka has no fed-back delete). Preview shows the exact command (cross-platform); runs
/// czkawka when available.
fn run_win_dupes(args: &[String]) -> ExitCode {
    // `--apply` is refused on EVERY platform, not just Windows. It used to be gated on
    // `is_windows()`, which made `win-dupes <dir> --apply` on macOS fold the write request into
    // an `ok:true` preview — a caller who asked for a mutation was told it succeeded when the
    // command cannot mutate anywhere. A write that cannot happen is an `unsupported` refusal,
    // the same envelope the other Windows-only commands give off-Windows.
    if args.iter().any(|a| a == "--apply") {
        return emit_failure(
            "win-dupes",
            "win-dupes --apply",
            "Windows duplicate discovery is read-only; dedupe/remove/link apply actions are macOS/fclones-only.",
        );
    }
    let dir = match args.iter().find(|a| !a.starts_with("--")) {
        Some(s) => s.clone(),
        None => return fail("win-dupes", "needs a directory"),
    };
    let scan = if args.iter().any(|a| a == "--images") {
        czkawka::Scan::SimilarImages
    } else {
        czkawka::Scan::Duplicates
    };
    let out_json = czkawka::report_path().to_string_lossy().into_owned();
    let planned = match czkawka::plan(&scan, &dir, &out_json) {
        Ok(p) => p,
        Err(e) => return fail("win-dupes", e),
    };
    match czkawka::resolve_czkawka() {
        Ok(bin) => match czkawka::execute(scan, &bin, &planned, &out_json) {
            Ok(v) => emit("win-dupes", &v.to_string(), false),
            Err(e) => fail("win-dupes", e),
        },
        Err(missing) => {
            // `ok` and `engine` are the ENVELOPE's — `data` does not restate them.
            let v = serde_json::json!({
                "scan": scan.as_str(),
                "available": false,
                "applied": false,
                "would_run": format!("czkawka_cli {}", planned.join(" ")),
                "args": planned,
                "reason": missing.message,
            });
            emit("win-dupes", &v.to_string(), false)
        }
    }
}

/// `burrow win-uninstall <app> [--confidence Good] [--apply]` — Windows uninstall via BCU.
/// Preview shows the exact BCU command (cross-platform); `--apply` runs it (Windows only).
fn run_win_uninstall(args: &[String]) -> ExitCode {
    // `--confidence` takes a value, so `win-uninstall --confidence Good "Foo App"` used to
    // uninstall an app called `Good`.
    let app = match positionals(args, &["--confidence"]).into_iter().next() {
        Some(s) => s,
        None => return fail("win-uninstall", "needs an app name"),
    };
    let confidence = flag_value(args, "--confidence").unwrap_or_else(|| "Good".to_string());
    let apply = args.iter().any(|a| a == "--apply");
    let planned = match bcu::plan(&app, &confidence, apply) {
        Ok(p) => p,
        Err(e) => return fail("win-uninstall", e),
    };
    if !apply {
        // Preview names the program that WOULD run — the $BURROW_BCU override verbatim when
        // set, else the canonical name. Deliberately NO resolution (no PATH walk, no stat):
        // the preview must work on machines without BCU and touch nothing.
        let program = std::env::var("BURROW_BCU").unwrap_or_else(|_| "BCU-console.exe".to_string());
        // `ok` and `engine` are the ENVELOPE's — `data` does not restate them.
        let v = serde_json::json!({
            "applied": false,
            "program": program,
            "would_run": format!("{program} {}", planned.join(" ")),
            "args": planned,
        });
        return emit("win-uninstall", &v.to_string(), false);
    }
    let bcu = match bcu::resolve_bcu() {
        Ok(bcu) => bcu,
        Err(e) => {
            // The details name the program that WOULD have run, so the failure is auditable
            // in the same shape as one BCU itself produced.
            let program =
                std::env::var("BURROW_BCU").unwrap_or_else(|_| "BCU-console.exe".to_string());
            let details =
                bcu::execution_details(std::path::Path::new(&program), &planned, None, "", "");
            return fail("win-uninstall", e.with_details(details));
        }
    };
    match bcu::execute(&bcu, &planned) {
        Ok(v) => emit("win-uninstall", &v.to_string(), false),
        Err(e) => fail("win-uninstall", e),
    }
}

/// `burrow trash <paths…> [--apply]` — cross-platform safe-delete to the Recycle Bin
/// (Windows) / Trash (macOS). Default preview; `--apply` performs the move.
///
/// `--apply-plan <file>` takes the paths from a held preview instead of the command line, and it
/// obeys the same rule: it PREVIEWS what it would trash until `--apply` is present. It used to
/// delete on sight — the only path in this binary where naming a file was itself the write —
/// which made the flag's name a lie in the direction that loses data, and left the launchd
/// template's `--apply-plan <file> --apply` passing a flag nothing read.
fn run_trash(args: &[String]) -> ExitCode {
    let apply = args.iter().any(|a| a == "--apply");
    // Scan once, execute the plan: `--apply-plan <file>` trashes exactly the paths
    // from a held preview, with no re-derivation of the deletion set.
    let paths = if let Some(plan_file) = flag_value(args, "--apply-plan") {
        let json = match std::fs::read_to_string(&plan_file) {
            Ok(s) => s,
            Err(e) => {
                return fail(
                    "trash",
                    output::Failure::io(format!("cannot read plan {plan_file}"), &e),
                )
            }
        };
        match recycle::plan_paths(&json) {
            Ok(p) => p,
            Err(e) => return fail("trash", e),
        }
    } else {
        match recycle::parse(args) {
            Ok((paths, _)) => paths,
            Err(e) => return fail("trash", e),
        }
    };
    if !apply {
        return emit("trash", &recycle::dry_run(&paths).to_string(), false);
    }
    if args.iter().any(|a| a == "--stream") {
        let failed = recycle::trash_each(&paths, &recycle::SystemTrash, |e| println!("{e}"));
        if failed > 0 {
            OK.with(|ok| ok.set(false));
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }
    match recycle::trash_all(&paths) {
        Ok(result) => emit("trash", &result.to_string(), false),
        Err(error) => fail("trash", error),
    }
}

/// `burrow gui [--install]` — launch the Burrow GUI app (installing the cask first if asked).
fn run_gui(args: &[String]) -> ExitCode {
    if !platform::is_macos() {
        return emit_failure(
            "gui",
            "gui",
            "The Burrow SwiftUI GUI launcher is macOS-only; the Windows branch currently ships the CLI.",
        );
    }
    let want_install = args.iter().any(|a| a == "--install");

    // Install if requested and not already present.
    if want_install && gui::locate().is_none() {
        if let Err(e) = gui::install() {
            return fail("gui", e);
        }
    }

    let path = match gui::locate() {
        Some(p) => p,
        None => {
            // Human hints on stderr; fail() adds the classified error envelope on stdout so
            // a GUI/agent caller parses the same shape as every other command.
            eprintln!("  install + launch:  burrow gui --install");
            eprintln!("  (runs: brew install --cask {})", gui::CASK);
            eprintln!("  or download:        {}", gui::SITE);
            return fail(
                "gui",
                output::Failure::not_found(format!(
                    "Burrow.app not found; run `burrow gui --install` or download from {}",
                    gui::SITE
                )),
            );
        }
    };

    if let Err(e) = gui::launch(&path) {
        return fail("gui", e);
    }
    let v = serde_json::json!({
        "launched": path.to_string_lossy(),
        "bundle_id": gui::BUNDLE_ID,
    });
    emit("gui", &v.to_string(), false)
}

/// The engine's `status` PAYLOAD (envelope already stripped by `engine::execute`), which is what
/// `metrics::sample_from_status` / `alerts_from_status` read: they look up `health_score`, `cpu`,
/// `disks` and `process_alerts` at the top level. Forwarding the engine's envelope here would
/// make every one of those lookups miss and fall through to its `unwrap_or` default — a health
/// score of -1 and 0% disk written into history with no error raised. See `engine::payload_of`.
fn engine_status() -> Result<String, output::Failure> {
    let bin = engine::resolve()?;
    let plan = engine::plan("status", &[])?;
    engine::execute_with_timeout(&bin, &plan, std::time::Duration::from_secs(60))
}

/// `burrow snapshot` — capture a status sample into the CLI-side history store.
fn run_snapshot(_args: &[String]) -> ExitCode {
    let json = match engine_status() {
        Ok(j) => j,
        Err(e) => return fail("snapshot", e),
    };
    let sample = match metrics::sample_from_status(&json, metrics::now_secs()) {
        Ok(s) => s,
        Err(e) => return fail("snapshot", e),
    };
    if let Err(e) = metrics::append_sample(&sample) {
        return fail("snapshot", e);
    }
    emit(
        "snapshot",
        &serde_json::to_string(&sample).unwrap_or_default(),
        false,
    )
}

/// `burrow digest [--days N]` — summarize recent snapshot history (weekly machine health).
///
/// Also `burrow report [--weekly]`, the plan's Phase 8 name, which the table aliases onto this
/// surface; `--weekly` is the default 7-day window and needs no handling.
fn run_digest(args: &[String]) -> ExitCode {
    let days: i64 = flag_value(args, "--days")
        .and_then(|s| s.parse().ok())
        .unwrap_or(7);
    let since = metrics::now_secs() - days * 86_400;
    let samples = metrics::load_samples(since);
    let v = match metrics::digest(&samples) {
        Some(d) => serde_json::to_value(d).unwrap_or(serde_json::Value::Null),
        None => serde_json::json!({
            "count": 0,
            "note": "no snapshots yet; run `burrow snapshot` periodically (cron/launchd)"
        }),
    };
    emit("digest", &v.to_string(), false)
}

/// `burrow watch` — surface leak/runaway process alerts from the engine.
fn run_watch(_args: &[String]) -> ExitCode {
    let json = match engine_status() {
        Ok(j) => j,
        Err(e) => return fail("watch", e),
    };
    let alerts = match metrics::alerts_from_status(&json) {
        Ok(a) => a,
        Err(e) => return fail("watch", e),
    };
    let v = serde_json::json!({ "count": alerts.len(), "alerts": alerts });
    emit("watch", &v.to_string(), false)
}

/// `burrow trigger [--threshold N]` — Storage-Sense-style decision: would a low-disk
/// auto-clean fire right now? (Read-only; reports the decision, does not clean.)
fn run_trigger(args: &[String]) -> ExitCode {
    let threshold: f64 = flag_value(args, "--threshold")
        .and_then(|s| s.parse().ok())
        .unwrap_or(85.0);
    let json = match engine_status() {
        Ok(j) => j,
        Err(e) => return fail("trigger", e),
    };
    let sample = match metrics::sample_from_status(&json, metrics::now_secs()) {
        Ok(s) => s,
        Err(e) => return fail("trigger", e),
    };
    let should = metrics::should_autoclean(sample.disk_used_percent, threshold);
    let mut v = serde_json::json!({
        "disk_used_percent": sample.disk_used_percent,
        "threshold": threshold,
        "should_clean": should,
    });
    // Phase 8 composition: `--rules <dir>` plans the AUTO-opted (risk:safe, conditions-met,
    // existing) targets when the trigger fires — the plan array is `trash --apply-plan`'s
    // bare-array input, so a launchd/cron job composes: trigger -> plan file -> apply.
    // Below threshold nothing is ever planned.
    if let Some(rules_dir) = flag_value(args, "--rules") {
        let (plan, skipped) = if should {
            match rules::auto_clean_plan(
                std::path::Path::new(&rules_dir),
                &platform::home_dir_string(),
            ) {
                Ok(x) => x,
                Err(e) => return fail("trigger", e),
            }
        } else {
            (Vec::new(), Vec::new())
        };
        v["planned"] = serde_json::json!(plan.len());
        v["plan"] = serde_json::json!(plan);
        // Named, not just counted: "which file is broken" is the only actionable form of this.
        if !skipped.is_empty() {
            v["skipped_rule_files"] = serde_json::json!(skipped);
        }
    }
    emit("trigger", &v.to_string(), false)
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// The caller's POSITIONAL arguments: the tokens that are neither a `--flag` nor a flag's value.
///
/// This is [`flag_value`]'s other half, and it exists because the spelling it replaces —
/// `args.iter().find(|a| !a.starts_with("--"))` — reads a value-taking flag's VALUE as the
/// positional. `sentinel --watch --interval-ms 500` picked `500` as the directory to watch, which
/// is exactly what the shipped `packaging/launchd/dev.caezium.burrow.sentinel.plist.template`
/// runs: the daemon polled a relative directory named `15000` and reported nothing, forever.
///
/// `value_flags` names the flags whose NEXT token belongs to them. Each caller passes the same
/// set it hands `flag_value`, so the two agree about what is a value and what is an argument.
fn positionals(args: &[String], value_flags: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut expecting_value = false;
    for a in args {
        if expecting_value {
            expecting_value = false;
            continue;
        }
        if a.starts_with("--") {
            expecting_value = value_flags.contains(&a.as_str());
            continue;
        }
        out.push(a.clone());
    }
    out
}

/// Which engine serves `cmd`, for the envelope's `engine` field — on EVERY envelope the command
/// can produce. [`emit`], [`fail`] and [`emit_failure`] all resolve it here, so a command cannot
/// name one engine when it works and another when it does not. That is not hypothetical
/// tidiness: the failure envelopes hardcoded `burrow-engine`, so `burrow slim` answered `native`
/// on success and `burrow-engine` on failure, and `win-uninstall` answered `bcu` on success and
/// `burrow-engine` on a BCU it could not resolve — in the very envelope whose `error.details`
/// said `"engine":"bcu"`.
///
/// It is read off the command table, the same row `run` dispatched from, so who is dispatched
/// and who is reported cannot disagree. See `commands::Owner` for why a missing sidecar still
/// reports the sidecar.
fn engine_for(cmd: &str) -> &'static str {
    commands::spec(cmd).map_or("native", |s| s.owner.label())
}

fn emit(cmd: &str, out: &str, raw: bool) -> ExitCode {
    if raw || RAW.with(|raw| raw.get()) {
        print!("{out}");
    } else {
        println!("{}", output::wrap(cmd, engine_for(cmd), out));
    }
    ExitCode::SUCCESS
}

/// The conductor's OWN platform refusal, raised before any engine runs: a FAILURE envelope
/// (top-level `ok:false`, kind `unsupported`) carrying the `feature` that is unavailable here.
/// A GUI branches on the outer `ok`, so an unsupported response must not look like `ok:true`.
fn emit_failure(cmd: &str, feature: &str, detail: &str) -> ExitCode {
    OK.with(|o| o.set(false));
    println!(
        "{}",
        output::unsupported_envelope(cmd, engine_for(cmd), feature, detail)
    );
    ExitCode::FAILURE
}

/// Report a failure. Takes anything that converts into an [`output::Failure`], so a plain
/// `String`/`&str` from a conductor-native command still reads as a one-liner (and classifies
/// as the generic `error`) while an already-classified failure — relayed out of the engine, or
/// raised with an explicit kind and process details — keeps its classification all the way here.
///
/// It resolves the envelope's `engine` from `cmd` through [`engine_for`], the way [`emit`] does,
/// which is what leaves all three dozen call sites unchanged: each already passes the command it
/// is failing, and the command is the whole input the attribution needs.
fn fail(cmd: &str, e: impl Into<output::Failure>) -> ExitCode {
    OK.with(|o| o.set(false));
    let failure = e.into();
    // Human-readable line on stderr for terminal use, plus the unified error
    // envelope on stdout so a GUI/agent parses one shape and branches on `ok`.
    eprintln!("burrow {cmd}: {}", failure.message);
    println!("{}", output::error_envelope(cmd, engine_for(cmd), &failure));
    ExitCode::FAILURE
}

/// Rendered from the command table, so a command's usage, description and attribution come from
/// the same row `run` dispatches on and `engine_for` reports.
fn help_text() -> String {
    let section = |which: commands::Section| -> String {
        commands::COMMANDS
            .iter()
            .filter(|s| s.section == which)
            .map(|s| format!("  {:<44} {} {}\n", s.usage, s.summary, s.help_tag()))
            .collect()
    };
    format!(
        "burrow {} - agent-native system-cleaning conductor\n\n\
         USAGE:\n  burrow <command> [flags]\n\n\
         READ-ONLY:\n{}\n\
         ACTIONS (default dry-run/preview; add --apply to execute):\n{}\n\
         META: version | help | telemetry [on|off|status]  (plain text, no envelope)\n\
         GLOBAL: --raw emits the bare payload without the envelope\n\
         ENGINE: $BURROW_ENGINE / $BURROW_ENGINE_DIR / beside burrow / sibling ../burrow-engine / PATH\n\
         SIDECARS: $BURROW_CZKAWKA / $BURROW_BCU / PATH",
        env!("CARGO_PKG_VERSION"),
        section(commands::Section::ReadOnly),
        section(commands::Section::Action),
    )
}

fn print_help() {
    println!("{}", help_text());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// Every table row is dispatched somewhere, and to exactly one place: the engine rows have no
    /// native handler (except `sentinel`, whose `--watch` half is the conductor's) and every other
    /// row has one. A row with two homes would be routed by whichever `run` checked first; a row
    /// with none would fall through to the "no handler" refusal on every call.
    #[test]
    fn every_command_has_exactly_one_home() {
        for spec in commands::COMMANDS {
            let native = native_handler(spec.name).is_some();
            let engine = engine::owns(spec.name);
            match (engine, native) {
                (true, true) => assert_eq!(
                    spec.name, "sentinel",
                    "{} is the engine's and must not also be handled here",
                    spec.name
                ),
                (true, false) | (false, true) => {}
                (false, false) => panic!("{} has no home", spec.name),
            }
            // Attribution follows the same row.
            assert_eq!(engine_for(spec.name), spec.owner.label());
            for alias in spec.aliases {
                assert_eq!(engine_for(alias), spec.owner.label(), "{alias}");
            }
        }
        assert!(native_handler("bogus").is_none());
    }

    /// The help text is the table: every command's usage appears once, tagged with the engine
    /// its envelopes will name. This is the drift that used to exist — `snapshot`/`watch`/
    /// `trigger` were labelled `[engine]` in help while `engine_for` said `native`.
    #[test]
    fn help_lists_every_command_with_its_real_attribution() {
        let help = help_text();
        for spec in commands::COMMANDS {
            let line = help
                .lines()
                .find(|l| l.trim_start().starts_with(spec.usage))
                .unwrap_or_else(|| panic!("{} missing from help", spec.name));
            assert!(
                line.ends_with(&spec.help_tag()),
                "{}: help says {line:?}, envelopes say {}",
                spec.name,
                engine_for(spec.name)
            );
        }
        assert!(help.contains("[native]"), "{help}");
        assert!(!help.contains("[engine]"), "stale tag: {help}");
    }

    /// A nonzero engine exit code must never be reported as a clean run.
    ///
    /// `code as u8` — the cast this replaced — keeps only the low byte, so 256 and 0x10000 became
    /// `ExitCode::from(0)`, i.e. SUCCESS. Unix masks the wait status to 8 bits before `.code()`
    /// ever sees it, so those values cannot arise there; a Windows exit code is a full 32-bit
    /// value and this conductor is one binary.
    #[test]
    fn a_failing_engine_is_never_reported_as_a_clean_run() {
        for code in [
            1,
            2,
            127,
            255,
            256,
            257,
            512,
            0x1_0000,
            i32::MAX,
            -1,
            i32::MIN,
        ] {
            assert_ne!(
                nonzero_exit_byte(code),
                0,
                "exit {code} must stay a failure"
            );
        }
    }

    /// A code that fits in a byte is relayed unchanged, so the engine's own number survives
    /// wherever `ExitCode` can carry it.
    #[test]
    fn a_representable_code_is_relayed_unchanged() {
        for code in 1..=255i32 {
            assert_eq!(nonzero_exit_byte(code), u8::try_from(code).unwrap());
        }
    }

    /// The token after a value-taking flag belongs to that flag, and is not the caller's
    /// positional argument. This is the shape the launchd sentinel template runs.
    #[test]
    fn a_value_taking_flags_value_is_not_a_positional() {
        let flags = &["--interval-ms", "--max-ticks"];
        assert!(positionals(&a(&["--watch", "--interval-ms", "500"]), flags).is_empty());
        assert_eq!(
            positionals(&a(&["--watch", "--interval-ms", "500", "/trash"]), flags),
            a(&["/trash"])
        );
        assert_eq!(
            positionals(&a(&["/trash", "--watch", "--max-ticks", "2"]), flags),
            a(&["/trash"])
        );
    }

    /// A flag NOT declared as value-taking does not swallow the token after it — that token is
    /// the positional. Over-consuming would lose the argument as surely as under-consuming
    /// mistook one for it.
    #[test]
    fn only_the_declared_value_flags_consume_their_successor() {
        assert_eq!(
            positionals(&a(&["--watch", "/trash"]), &["--interval-ms"]),
            a(&["/trash"])
        );
        assert_eq!(
            positionals(&a(&["--apply", "Foo App"]), &["--confidence"]),
            a(&["Foo App"])
        );
    }

    /// A value-taking flag in final position has no value to consume, and must not reach past the
    /// end of the argument list.
    #[test]
    fn a_trailing_value_flag_consumes_nothing() {
        assert!(positionals(&a(&["--interval-ms"]), &["--interval-ms"]).is_empty());
        assert_eq!(
            positionals(&a(&["/trash", "--interval-ms"]), &["--interval-ms"]),
            a(&["/trash"])
        );
    }
}
