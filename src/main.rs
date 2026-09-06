//! Burrow CLI — agent-native system-cleaning conductor.
//!
//! Dispatches to burrow-engine (clean/uninstall/optimize/purge/installer/history via the bash
//! entrypoint; status/analyze via the Go binaries), the fclones sidecar (dupes), and the
//! declarative rule engine (rules). Every result is wrapped in a stable Burrow envelope.
//! Destructive commands default to dry-run/preview; `--apply` executes (explain-before-delete).

mod bcu;
mod czkawka;
mod dupes;
mod engine;
mod evict;
mod gui;
mod macho;
mod metrics;
mod net;
mod orphan;
mod output;
mod photos;
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
    if !matches!(
        cmd.as_str(),
        "help" | "--help" | "-h" | "version" | "--version" | "-V"
    ) {
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
            ExitCode::SUCCESS
        }
        "help" | "--help" | "-h" => {
            print_help();
            ExitCode::SUCCESS
        }
        c @ ("status" | "analyze" | "clean" | "optimize" | "purge" | "uninstall" | "installer"
        | "history") => run_engine(c, &args[1..]),
        "dupes" => run_dupes(&args[1..]),
        "rules" => run_rules(&args[1..]),
        "orphans" => run_orphans(&args[1..]),
        "diff" => run_diff(&args[1..]),
        "snapshot" => run_snapshot(&args[1..]),
        "digest" => run_digest(&args[1..]),
        "watch" => run_watch(&args[1..]),
        "trigger" => run_trigger(&args[1..]),
        "slim-check" => run_slim_check(&args[1..]),
        "slim" => run_slim(&args[1..]),
        "evict" => run_evict(&args[1..]),
        "gui" | "app" => run_gui(&args[1..]),
        "trash" => run_trash(&args[1..]),
        "win-uninstall" => run_win_uninstall(&args[1..]),
        "win-dupes" => run_win_dupes(&args[1..]),
        "photos" => run_photos(&args[1..]),
        "net" => run_net(&args[1..]),
        "sentinel" => run_sentinel(&args[1..]),
        other => {
            eprintln!("burrow: unknown command '{other}'\n");
            print_help();
            ExitCode::FAILURE
        }
    }
}

/// Resolve the engine and run the command. TTY-aware, like mo itself: in an interactive
/// terminal the engine's native TUI / colored output is shown; piped (or with `--json`/`--raw`)
/// it emits the stable JSON envelope for agents.
fn run_engine(cmd: &str, args: &[String]) -> ExitCode {
    use std::io::IsTerminal;

    // Live NDJSON stream: `burrow status --watch` passes straight through to the engine's
    // streaming feed (no envelope — an infinite stream can't be wrapped). Ctrl-C to stop.
    if cmd == "status" && args.iter().any(|a| a == "--watch") {
        let dir = match engine::resolve_dir() {
            Ok(d) => d,
            Err(e) => return fail(cmd, e),
        };
        let mut wargs = vec!["--watch".to_string()];
        if let Some(iv) = flag_value(args, "--watch-interval") {
            wargs.push("--watch-interval".into());
            wargs.push(iv);
        }
        let plan = engine::Plan {
            target: engine::Target::GoBinary("status-go"),
            args: wargs,
        };
        return match engine::execute_native(&dir, &plan) {
            Ok(0) => ExitCode::SUCCESS,
            Ok(code) => ExitCode::from(code as u8),
            Err(e) => fail(cmd, e),
        };
    }

    // Live NDJSON stream: `burrow analyze --progress` passes the engine's scan-progress
    // feed straight through (no envelope), like `status --watch` — a GUI renders the
    // treemap as it fills in instead of blocking on the whole tree.
    if cmd == "analyze" && args.iter().any(|a| a == "--progress") {
        let dir = match engine::resolve_dir() {
            Ok(d) => d,
            Err(e) => return fail(cmd, e),
        };
        let mut pargs = vec!["--json".to_string(), "--progress".to_string()];
        for a in args {
            if !a.starts_with("--") {
                pargs.push(a.clone()); // optional positional <path>
            }
        }
        let plan = engine::Plan {
            target: engine::Target::GoBinary("analyze-go"),
            args: pargs,
        };
        return match engine::execute_native(&dir, &plan) {
            Ok(0) => ExitCode::SUCCESS,
            Ok(code) => ExitCode::from(code as u8),
            Err(e) => fail(cmd, e),
        };
    }

    let raw = args.iter().any(|a| a == "--raw");
    let force_json = args.iter().any(|a| a == "--json");
    let native = !raw && !force_json && std::io::stdout().is_terminal();

    let mut plan = match engine::plan(cmd, args) {
        Ok(p) => p,
        Err(e) => return fail(cmd, e),
    };
    let dir = match engine::resolve_dir() {
        Ok(d) => d,
        Err(e) => return fail(cmd, e),
    };

    if native {
        // Drop `--json` so status/analyze render their TUI; clean/etc. show colored output.
        plan.args.retain(|a| a != "--json");
        return match engine::execute_native(&dir, &plan) {
            Ok(0) => ExitCode::SUCCESS,
            Ok(code) => ExitCode::from(code as u8),
            Err(e) => fail(cmd, e),
        };
    }
    match engine::execute(&dir, &plan) {
        Ok(out) => emit(cmd, &out, raw),
        Err(e) => fail(cmd, e),
    }
}

/// `burrow dupes [group|dedupe|remove|link] <paths…> [--apply]` via the fclones sidecar.
fn run_dupes(args: &[String]) -> ExitCode {
    let raw = args.iter().any(|a| a == "--raw");
    let (sub, rest): (&str, &[String]) = match args.first().map(String::as_str) {
        Some(s @ ("group" | "dedupe" | "remove" | "link")) => (s, &args[1..]),
        _ => ("group", args),
    };
    let plan = match dupes::plan(sub, rest) {
        Ok(p) => p,
        Err(e) => return fail("dupes", e),
    };
    if platform::is_windows() && std::env::var("BURROW_FCLONES").is_err() {
        if args.iter().any(|a| a == "--apply") {
            return emit_failure(
                "dupes",
                platform::unsupported(
                    "dupes apply",
                    "Windows does not support APFS clonefile dedupe/link/remove through fclones; use win-dupes/czkawka for discovery first.",
                ),
            );
        }
        return run_czkawka_duplicates("dupes", rest, raw);
    }
    let fclones = match dupes::resolve_fclones() {
        Ok(b) => b,
        Err(e) => return fail("dupes", e),
    };
    match dupes::execute(&fclones, &plan) {
        Ok(out) => emit("dupes", &out, raw),
        Err(e) => fail("dupes", e),
    }
}

/// `burrow rules [list|validate|dryrun] [dir] [--app <bundle-id>]`.
fn run_rules(args: &[String]) -> ExitCode {
    let sub = args.first().map(String::as_str).unwrap_or("list");
    let dir_arg = args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "rules".to_string());
    let dir = std::path::PathBuf::from(&dir_arg);
    let loaded = match rules::load_dir(&dir) {
        Ok(l) => l,
        Err(e) => return fail("rules", e),
    };

    match sub {
        "list" => {
            let apps: Vec<_> = loaded
                .iter()
                .filter_map(|l| {
                    l.result
                        .as_ref()
                        .ok()
                        .map(|rf| build_app_summary(&l.file, rf))
                })
                .collect();
            let v = serde_json::json!({ "count": apps.len(), "apps": apps });
            emit("rules", &v.to_string(), false)
        }
        "validate" => {
            let mut problems = Vec::new();
            let mut valid = 0usize;
            for l in &loaded {
                match &l.result {
                    Ok(rf) => {
                        let errs = rules::validate(rf);
                        if errs.is_empty() {
                            valid += 1;
                        } else {
                            problems.push(serde_json::json!({ "file": l.file, "errors": errs }));
                        }
                    }
                    Err(e) => problems.push(serde_json::json!({ "file": l.file, "errors": [e] })),
                }
            }
            let ok = problems.is_empty();
            let v =
                serde_json::json!({ "files": loaded.len(), "valid": valid, "problems": problems });
            println!("{}", output::envelope("rules", &v.to_string()));
            if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        "dryrun" => {
            let app_filter = args
                .iter()
                .position(|a| a == "--app")
                .and_then(|i| args.get(i + 1))
                .cloned();
            let home = platform::home_dir_string();
            let mut items = Vec::new();
            for l in &loaded {
                let Ok(rf) = &l.result else { continue };
                if let Some(f) = &app_filter {
                    if !rf.app.bundle_ids.iter().any(|b| b == f) {
                        continue;
                    }
                }
                for r in &rf.rules {
                    for t in &r.targets {
                        let p = rules::expand_path(&t.path, &home);
                        let exists = std::path::Path::new(&p).exists();
                        items.push(serde_json::json!({
                            "app": rf.app.name,
                            "rule": r.id,
                            "risk": r.risk.as_str(),
                            "default_selected": rules::default_selected(r),
                            "method": r.action.method.as_str(),
                            "path": p,
                            "exists": exists,
                        }));
                    }
                }
            }
            let v = serde_json::json!({ "items": items });
            emit("rules", &v.to_string(), false)
        }
        other => fail(
            "rules",
            format!("unknown subcommand '{other}' (list|validate|dryrun)"),
        ),
    }
}

/// `burrow orphans <dir> [--installed id1,id2,…]` — read-only leftover scan.
/// Without `--installed`, enumerates /Applications.
fn run_orphans(args: &[String]) -> ExitCode {
    let root = args.iter().find(|a| !a.starts_with("--")).cloned();
    let installed = match flag_value(args, "--installed") {
        Some(csv) => orphan::installed_from_cli_csv(&csv),
        None => orphan::enumerate_installed_apps(),
    };
    let inventory_sources = orphan::inventory_source_counts(&installed);
    let roots: Vec<String> = match root {
        Some(r) => vec![r],
        None if platform::is_windows() => orphan::default_scan_roots()
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        None => return fail("orphans", "needs a directory to scan".into()),
    };
    let mut hits = Vec::new();
    for root in &roots {
        hits.extend(orphan::scan(std::path::Path::new(root), &installed));
    }
    let v = serde_json::json!({
        "roots": roots,
        "installed_count": installed.len(),
        "inventory_sources": inventory_sources,
        "count": hits.len(),
        "orphans": hits,
    });
    emit("orphans", &v.to_string(), false)
}

/// `burrow diff <dir>` — disk-growth diff vs the previous scan of the same dir (first run
/// saves a baseline). Read-only except for the saved scan file.
fn run_diff(args: &[String]) -> ExitCode {
    let root = match args.iter().find(|a| !a.starts_with("--")) {
        Some(r) => r.clone(),
        None => return fail("diff", "needs a directory to scan".into()),
    };
    let curr = treediff::scan_tree(std::path::Path::new(&root), 4);
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

/// `burrow slim-check <binary>` — read-only Mach-O fat analysis: arch slices + bytes a
/// thin-to-host-arch would reclaim. (Thinning + ad-hoc re-sign is the deferred write step.)
fn run_slim_check(args: &[String]) -> ExitCode {
    use std::io::Read;
    let path = match args.iter().find(|a| !a.starts_with("--")) {
        Some(p) => p.clone(),
        None => return fail("slim-check", "needs a path to a Mach-O binary".into()),
    };
    if platform::is_windows() {
        return emit_failure(
            "slim-check",
            platform::unsupported(
                "slim-check",
                "Mach-O universal binary thinning is macOS-only; Windows PE analysis is not implemented in this branch.",
            ),
        );
    }
    let mut buf = vec![0u8; 4096];
    let read = match std::fs::File::open(&path).and_then(|mut f| f.read(&mut buf)) {
        Ok(n) => n,
        Err(e) => return fail("slim-check", format!("cannot read {path}: {e}")),
    };
    buf.truncate(read);
    let slices = match macho::parse_fat(&buf) {
        Ok(s) => s,
        Err(e) => return fail("slim-check", e),
    };
    let keep = macho::host_cputype();
    let v = serde_json::json!({
        "path": path,
        "arch_count": slices.len(),
        "host_keep_cputype": keep,
        "potential_savings_bytes": macho::slim_savings(&slices, keep),
        "slices": slices,
    });
    emit("slim-check", &v.to_string(), false)
}

/// `burrow slim <binary> [--apply --output <path>]` — thin a fat Mach-O to the host arch and
/// ad-hoc re-sign. Default reports; `--apply` writes (requires `--output`, never in-place).
fn run_slim(args: &[String]) -> ExitCode {
    let path = match args.iter().find(|a| !a.starts_with("--")) {
        Some(p) => p.clone(),
        None => return fail("slim", "needs a path to a Mach-O binary".into()),
    };
    if platform::is_windows() {
        return emit_failure(
            "slim",
            platform::unsupported(
                "slim",
                "Mach-O thinning and ad-hoc codesign are macOS-only; Windows slimming is intentionally disabled.",
            ),
        );
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => return fail("slim", format!("cannot read {path}: {e}")),
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
                "--apply needs --output <path> (in-place thinning is not offered)".into(),
            )
        }
    };
    let thinned = match macho::thin(&bytes, keep) {
        Ok(t) => t,
        Err(e) => return fail("slim", e),
    };
    if let Err(e) = std::fs::write(&output, &thinned) {
        return fail("slim", format!("write {output}: {e}"));
    }
    if let Ok(meta) = std::fs::metadata(&path) {
        let _ = std::fs::set_permissions(&output, meta.permissions()); // preserve +x
    }
    let resigned = std::process::Command::new("codesign")
        .args(["--force", "--sign", "-", &output])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let v = serde_json::json!({
        "input": path, "output": output,
        "original_size": bytes.len(), "thinned_size": thinned.len(),
        "reclaimed_bytes": bytes.len().saturating_sub(thinned.len()),
        "resigned_adhoc": resigned, "applied": true,
    });
    emit("slim", &v.to_string(), false)
}

/// `burrow net [--limit N]` — per-app network usage (who's phoning home) via nettop.
fn run_net(args: &[String]) -> ExitCode {
    let limit: usize = flag_value(args, "--limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(15);
    let mut rows = match net::collect() {
        Ok(o) => o,
        Err(e) => {
            return emit_failure(
                "net",
                platform::unsupported("net", &format!("per-app network unavailable: {e}")),
            )
        }
    };
    rows.truncate(limit);
    let v = serde_json::json!({ "count": rows.len(), "by_total_bytes": rows });
    emit("net", &v.to_string(), false)
}

/// `burrow sentinel [trashdir]` — `.app` bundles currently in the Trash (leftover candidates).
fn run_sentinel(args: &[String]) -> ExitCode {
    let dir = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| format!("{}/.Trash", platform::home_dir_string()));
    if platform::is_windows() && args.iter().all(|a| a.starts_with("--")) {
        return emit_failure(
            "sentinel",
            platform::unsupported(
                "sentinel",
                "Trash sentinel watches macOS .app bundles; Windows Recycle Bin monitoring is not implemented.",
            ),
        );
    }
    let apps = sentinel::scan_trash(std::path::Path::new(&dir));
    let v = serde_json::json!({ "trash": dir, "count": apps.len(), "trashed_apps": apps });
    emit("sentinel", &v.to_string(), false)
}

/// `burrow photos <dir> [--threshold N]` — find visually-similar PNG/JPEG images.
fn run_photos(args: &[String]) -> ExitCode {
    let dir = match args.iter().find(|a| !a.starts_with("--")) {
        Some(d) => d.clone(),
        None => return fail("photos", "needs a directory to scan".into()),
    };
    let threshold: u32 = flag_value(args, "--threshold")
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let groups = photos::scan_dir(std::path::Path::new(&dir), threshold);
    let v = serde_json::json!({ "dir": dir, "threshold": threshold, "similar_groups": groups });
    emit("photos", &v.to_string(), false)
}

/// `burrow win-dupes <dir> [--images]` — Windows duplicates / similar photos via czkawka_cli.
/// Preview shows the exact command (cross-platform); runs czkawka when available.
fn run_win_dupes(args: &[String]) -> ExitCode {
    let dir = match args.iter().find(|a| !a.starts_with("--")) {
        Some(s) => s.clone(),
        None => return fail("win-dupes", "needs a directory".into()),
    };
    let scan = if args.iter().any(|a| a == "--images") {
        czkawka::Scan::SimilarImages
    } else {
        czkawka::Scan::Duplicates
    };
    let out_json = std::env::temp_dir()
        .join("burrow-czkawka.json")
        .to_string_lossy()
        .into_owned();
    let planned = match czkawka::plan(&scan, &dir, &out_json) {
        Ok(p) => p,
        Err(e) => return fail("win-dupes", e),
    };
    match czkawka::resolve_czkawka() {
        Ok(bin) => match czkawka::execute(&bin, &planned, &out_json) {
            Ok(v) => emit("win-dupes", &v.to_string(), false),
            Err(e) => fail("win-dupes", e),
        },
        Err(_) => {
            // czkawka not installed here → show the exact command we'd run (cross-platform).
            let v = serde_json::json!({
                "available": false,
                "would_run": format!("czkawka_cli {}", planned.join(" ")),
                "args": planned,
            });
            emit("win-dupes", &v.to_string(), false)
        }
    }
}

/// `burrow win-uninstall <app> [--confidence Good] [--apply]` — Windows uninstall via BCU.
/// Preview shows the exact BCU command (cross-platform); `--apply` runs it (Windows only).
fn run_czkawka_duplicates(command: &str, args: &[String], raw: bool) -> ExitCode {
    let dir = match args.iter().find(|a| !a.starts_with("--")) {
        Some(s) => s.clone(),
        None => return fail(command, "needs a directory".into()),
    };
    let scan = if args.iter().any(|a| a == "--images") {
        czkawka::Scan::SimilarImages
    } else {
        czkawka::Scan::Duplicates
    };
    let out_json = std::env::temp_dir()
        .join("burrow-czkawka.json")
        .to_string_lossy()
        .into_owned();
    let planned = match czkawka::plan(&scan, &dir, &out_json) {
        Ok(p) => p,
        Err(e) => return fail(command, e),
    };
    match czkawka::resolve_czkawka() {
        Ok(bin) => match czkawka::execute(&bin, &planned, &out_json) {
            Ok(v) => emit(command, &v.to_string(), raw),
            Err(e) => fail(command, e),
        },
        Err(_) => {
            let v = serde_json::json!({
                "available": false,
                "would_run": format!("czkawka_cli {}", planned.join(" ")),
                "args": planned,
            });
            emit(command, &v.to_string(), raw)
        }
    }
}

fn run_win_uninstall(args: &[String]) -> ExitCode {
    let app = match args.iter().find(|a| !a.starts_with("--")) {
        Some(s) => s.clone(),
        None => return fail("win-uninstall", "needs an app name".into()),
    };
    let confidence = flag_value(args, "--confidence").unwrap_or_else(|| "Good".to_string());
    let apply = args.iter().any(|a| a == "--apply");
    let planned = match bcu::plan(&app, &confidence, apply) {
        Ok(p) => p,
        Err(e) => return fail("win-uninstall", e),
    };
    if !apply {
        let v = serde_json::json!({
            "applied": false,
            "would_run": format!("BCU-console.exe {}", planned.join(" ")),
            "args": planned,
        });
        return emit("win-uninstall", &v.to_string(), false);
    }
    let v = match bcu::resolve_bcu().and_then(|b| bcu::execute(&b, &planned)) {
        Ok(v) => v,
        Err(e) => return fail("win-uninstall", e),
    };
    emit("win-uninstall", &v.to_string(), false)
}

/// `burrow trash <paths…> [--apply]` — cross-platform safe-delete to the Recycle Bin
/// (Windows) / Trash (macOS). Default preview; `--apply` performs the move.
fn run_trash(args: &[String]) -> ExitCode {
    // Scan once, execute the plan: `--apply-plan <file>` trashes exactly the paths
    // from a held preview, with no re-derivation of the deletion set.
    if let Some(plan_file) = flag_value(args, "--apply-plan") {
        let json = match std::fs::read_to_string(&plan_file) {
            Ok(s) => s,
            Err(e) => return fail("trash", format!("cannot read plan {plan_file}: {e}")),
        };
        let paths = match recycle::plan_paths(&json) {
            Ok(p) => p,
            Err(e) => return fail("trash", e),
        };
        // --stream: emit one NDJSON event per item as it is trashed (live progress),
        // instead of a single batch envelope, so a GUI renders it as it happens.
        if args.iter().any(|a| a == "--stream") {
            recycle::trash_each(&paths, &recycle::SystemTrash, |e| println!("{e}"));
            return ExitCode::SUCCESS;
        }
        return emit("trash", &recycle::trash_all(&paths).to_string(), false);
    }
    let (paths, apply) = match recycle::parse(args) {
        Ok(x) => x,
        Err(e) => return fail("trash", e),
    };
    let v = if apply {
        recycle::trash_all(&paths)
    } else {
        recycle::dry_run(&paths)
    };
    emit("trash", &v.to_string(), false)
}

/// `burrow gui [--install]` — launch the Burrow GUI app (installing the cask first if asked).
fn run_gui(args: &[String]) -> ExitCode {
    if !platform::is_macos() {
        return emit_failure(
            "gui",
            platform::unsupported(
                "gui",
                "The Burrow SwiftUI GUI launcher is macOS-only; the Windows branch currently ships the CLI.",
            ),
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
            eprintln!("burrow gui: Burrow.app is not installed.");
            eprintln!("  install + launch:  burrow gui --install");
            eprintln!("  (runs: brew install --cask {})", gui::CASK);
            eprintln!("  or download:        {}", gui::SITE);
            return ExitCode::FAILURE;
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

/// `burrow evict <paths…> [--apply]` — iCloud dehydration via `brctl` (default preview).
fn run_evict(args: &[String]) -> ExitCode {
    let (paths, apply) = match evict::parse(args) {
        Ok(x) => x,
        Err(e) => return fail("evict", e),
    };
    let v = if apply {
        match evict::execute_apply(&paths) {
            Ok(v) => v,
            Err(e) => return fail("evict", e),
        }
    } else {
        evict::dry_run(&paths)
    };
    emit("evict", &v.to_string(), false)
}

fn engine_status() -> Result<String, String> {
    let dir = engine::resolve_dir()?;
    let plan = engine::plan("status", &[])?;
    engine::execute_with_timeout(&dir, &plan, std::time::Duration::from_secs(60))
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
    let v = serde_json::json!({
        "disk_used_percent": sample.disk_used_percent,
        "threshold": threshold,
        "should_clean": should,
    });
    emit("trigger", &v.to_string(), false)
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn build_app_summary(file: &str, rf: &rules::RuleFile) -> serde_json::Value {
    let rules_json: Vec<_> = rf
        .rules
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "category": r.category,
                "risk": r.risk.as_str(),
                "default_selected": rules::default_selected(r),
            })
        })
        .collect();
    serde_json::json!({
        "file": file,
        "app": rf.app.name,
        "bundle_ids": rf.app.bundle_ids,
        "provenance": rf.provenance.source,
        "rules": rules_json,
    })
}

fn emit(cmd: &str, out: &str, raw: bool) -> ExitCode {
    if raw {
        print!("{out}");
    } else {
        println!("{}", output::wrap(cmd, out));
    }
    ExitCode::SUCCESS
}

fn emit_failure(cmd: &str, v: serde_json::Value) -> ExitCode {
    OK.with(|o| o.set(false));
    // The value carries feature+detail (from platform::unsupported); emit it as a
    // proper FAILURE envelope (top-level ok:false), not the success wrapper — a GUI
    // branches on the outer `ok`, so an unsupported response must not look like ok:true.
    let feature = v.get("feature").and_then(|f| f.as_str()).unwrap_or(cmd);
    let detail = v.get("detail").and_then(|d| d.as_str()).unwrap_or("");
    println!("{}", output::unsupported_envelope(cmd, feature, detail));
    ExitCode::FAILURE
}

fn fail(cmd: &str, e: String) -> ExitCode {
    OK.with(|o| o.set(false));
    // Human-readable line on stderr for terminal use, plus the unified error
    // envelope on stdout so a GUI/agent parses one shape and branches on `ok`.
    eprintln!("burrow {cmd}: {e}");
    println!("{}", output::error_envelope(cmd, &e));
    ExitCode::FAILURE
}

fn help_text() -> String {
    format!(
        "burrow {} - agent-native system-cleaning conductor\n\n\
         USAGE:\n  burrow <command> [flags]\n\n\
         READ-ONLY:\n  \
         status [--raw|--watch]    System snapshot; --watch streams live NDJSON [engine]\n  \
         analyze [path] [--raw]    Disk-usage tree [engine]\n  \
         history [--limit N]       Cleanup history [engine]\n  \
         dupes <paths...>          Find duplicate files [mac: fclones, win: czkawka]\n  \
         rules [list|validate|dryrun] [dir] [--app id]   Declarative cleaning rules [cross-platform]\n  \
         orphans [dir] [--installed ids]   Leftover files belonging to no installed app [cross-platform]\n  \
         diff <dir>                Disk-growth since the previous scan of <dir> [cross-platform]\n  \
         snapshot                  Record a health sample to history [engine]\n  \
         digest [--days N]         Summarize recent health history [cross-platform]\n  \
         watch                     Leak/runaway process alerts [engine]\n  \
         net [--limit N]           Per-app network usage [mac: nettop, win: netstat fallback]\n  \
         sentinel [trashdir]       .app bundles in the Trash [macOS-only]\n  \
         trigger [--threshold N]   Would low-disk auto-clean fire now? [engine]\n  \
         slim-check <binary>       Mach-O fat slices + reclaimable bytes [macOS-only]\n  \
         slim <binary> [--apply --output P]   Thin a fat binary to host arch + re-sign [macOS-only]\n  \
         evict <paths...> [--apply]  Cloud-file dehydration [mac: brctl, win: OneDrive attrib]\n  \
         trash <paths...> [--apply]  Safe-delete to Recycle Bin (Win) / Trash (mac) [cross-platform]\n  \
         win-uninstall <app> [--confidence G] [--apply]   Windows uninstall via BCU [Windows]\n  \
         win-dupes <dir> [--images]   Windows duplicates / similar photos via czkawka [Windows]\n  \
         photos <dir> [--threshold N]   Visually-similar PNG/JPEG images [cross-platform]\n  \
         gui [--install]           Launch the Burrow GUI app (or install the cask) [macOS-only]\n  \
         telemetry [on|off|status] Anonymous usage analytics (opt-out)\n\n\
         ACTIONS (default dry-run/preview; add --apply to execute):\n  \
         clean [--apply]           Cache/junk cleanup\n  \
         optimize [--apply]        System cache refresh\n  \
         purge [--apply]           Project build-artifact cleanup\n  \
         uninstall <app> [--apply] Remove an app + leftovers\n  \
         installer [--apply]       Remove leftover installer files\n  \
         dupes dedupe <paths...> [--apply]   APFS clone-dedupe (macOS/fclones only)\n\n\
         GLOBAL: --raw | version | help\n  \
         ENGINE: $BURROW_ENGINE_DIR / sibling ../burrow-engine | sidecars: $BURROW_FCLONES / $BURROW_CZKAWKA / $BURROW_BCU / PATH",
        env!("CARGO_PKG_VERSION")
    )
}

fn print_help() {
    if std::env::var_os("BURROW_LEGACY_HELP").is_none() {
        println!("{}", help_text());
    } else {
        println!(
            "burrow {} — agent-native system-cleaning conductor\n\n\
         USAGE:\n  burrow <command> [flags]\n\n\
         READ-ONLY:\n  \
         status [--raw|--watch]    System snapshot; --watch streams live NDJSON\n  \
         analyze [path] [--raw]    Disk-usage tree\n  \
         history [--limit N]       Cleanup history\n  \
         dupes <paths…>            Find duplicate files (fclones)\n  \
         rules [list|validate|dryrun] [dir] [--app id]   Declarative cleaning rules\n  \
         orphans <dir> [--installed ids]   Leftover files belonging to no installed app\n  \
         diff <dir>                Disk-growth since the previous scan of <dir>\n  \
         snapshot                  Record a health sample to history\n  \
         digest [--days N]         Summarize recent health history\n  \
         watch                     Leak/runaway process alerts\n  \
         net [--limit N]           Per-app network usage (who's phoning home)\n  \
         sentinel [trashdir]       .app bundles in the Trash (leftover candidates)\n  \
         trigger [--threshold N]   Would low-disk auto-clean fire now?\n  \
         slim-check <binary>       Mach-O fat slices + reclaimable bytes\n  \
         slim <binary> [--apply --output P]   Thin a fat binary to host arch + re-sign\n  \
         evict <paths…> [--apply]  iCloud dehydration (free local space, reversible)\n  \
         trash <paths…> [--apply]  Safe-delete to Recycle Bin (Win) / Trash (mac)\n  \
         win-uninstall <app> [--confidence G] [--apply]   Windows uninstall via BCU\n  \
         win-dupes <dir> [--images]   Windows duplicates / similar photos via czkawka\n  \
         photos <dir> [--threshold N]   Visually-similar PNG/JPEG images\n  \
         gui [--install]           Launch the Burrow GUI app (or install the cask)\n  \
         telemetry [on|off|status] Anonymous usage analytics (opt-out)\n\n\
         ACTIONS (default dry-run/preview; add --apply to execute):\n  \
         clean [--apply]           Cache/junk cleanup\n  \
         optimize [--apply]        System cache refresh\n  \
         purge [--apply]           Project build-artifact cleanup\n  \
         uninstall <app> [--apply] Remove an app + leftovers\n  \
         installer [--apply]       Remove leftover installer files\n  \
         dupes dedupe <paths…> [--apply]   APFS clone-dedupe (reclaim without deleting)\n\n\
         GLOBAL: --raw · version · help\n  \
         ENGINE: $BURROW_ENGINE_DIR / sibling ../burrow-engine · fclones: $BURROW_FCLONES / PATH",
            env!("CARGO_PKG_VERSION")
        );
    }
}
