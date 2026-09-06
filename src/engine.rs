//! Engine resolution, command planning, and invocation.
//!
//! burrow-engine is invoked as a separate process (arm's-length): nothing from it is linked
//! into the conductor. Both components use FSL-1.1-ALv2 and are independently replaceable.
//! `plan()` is the pure, unit-tested core that maps a conductor command to an engine call.
//!
//! # The engine here is the Rust `burrow-engine`, not the bash+Go digger
//!
//! This module used to drive the digger: a bash `mole` entrypoint plus two Go binaries under
//! `bin/`. The GUI was repointed onto the single Rust `burrow-engine` binary; this is the
//! matching move for the standalone CLI, so both surfaces run the same engine.
//!
//! The two conventions are INVERTED on the only axis that destroys data. `mo clean` runs LIVE
//! and `--dry-run` previews it; `burrow-engine clean` previews and `--apply` runs it. Read
//! [`plan`] for what this module does about that.

use crate::commands::{self, Owner, Spec, Writes};
use crate::output::Failure;
use crate::platform;
use std::path::{Path, PathBuf};

/// A fully-resolved plan for one conductor command: the exact argv passed to the engine.
///
/// There is no longer a `Target`. The digger was three programs behind one directory (a bash
/// entrypoint and two Go binaries, picked per command); the engine is one binary that serves
/// every command, so the plan is just its argument vector.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub args: Vec<String>,
}

/// Whether the engine serves `command` — the single routing decision, read off the command
/// table so a caller never has to re-derive it.
///
/// `sentinel --watch` is the one case a caller must qualify further: the engine has no watch mode
/// (it refuses the flag rather than answering a streaming caller with one snapshot), so the
/// poll-based daemon stays here. `main.rs`'s `run_sentinel` makes that call.
pub fn owns(command: &str) -> bool {
    commands::spec(command).is_some_and(|s| s.owner == Owner::Engine)
}

/// The table row for an engine-served command.
fn engine_spec(command: &str) -> Result<&'static Spec, String> {
    match commands::spec(command) {
        Some(spec) if spec.owner == Owner::Engine => Ok(spec),
        _ => Err(format!("unknown command '{command}'")),
    }
}

/// Conductor-level flags that are consumed here and never forwarded verbatim.
///
/// Each is stripped for a specific reason, and the reasons are no longer the same:
///
/// - `--json` selects the envelope over a TTY rendering. The engine has exactly one output mode
///   and emits JSON regardless, so forwarding it would be a no-op; it stays a conductor concern.
/// - `--raw` asks for the payload WITHOUT the conductor's envelope. The engine REFUSES it
///   outright (exit 2, "unknown … option: --raw" — it has one output mode and won't pretend
///   otherwise), so stripping it here is load-bearing rather than tidiness: forwarding it turns
///   every `--raw` invocation into a hard failure.
///
/// `--stream` stays an engine argument. On supported commands [`plan`] reorders it alongside
/// the explicit write guard; elsewhere the engine must see and refuse the unsupported flag.
///
/// `--apply` is deliberately NOT in this list, because it is the opposite kind of flag: it
/// requests a WRITE. It is stripped only for the commands whose row says [`plan`] re-emits it —
/// so on a read-only command it reaches the engine and is refused ("names a MUTATION the command
/// does not have"). Swallowing it there would tell a caller a write was requested and honoured
/// when nothing could ever be written.
fn is_conductor_flag(a: &str) -> bool {
    matches!(a, "--json" | "--raw")
}

/// Map a burrow command + user args to an engine invocation.
///
/// # The safety model, and why the preview says so out loud
///
/// The conductor's contract is explain-before-delete: destructive commands preview by default
/// and `--apply` executes. The engine's own default is the same, so the naive translation for a
/// preview is to send NOTHING — the bare `["clean"]` — and let the engine's default carry it.
///
/// That is what the app first did, and it is a trap here in a way it is not there. [`resolve`]
/// finds the engine by walking up to a sibling checkout, and this module previously accepted a
/// binary named `mole` in that position. Against the digger, a bare `["clean"]` is the LIVE run.
/// So a build that resolved a legacy entrypoint while planning in engine dialect would turn
/// every preview into a real deletion — the inversion pointing the dangerous way.
///
/// Two things close that, and both are here because either alone is a default away from failing:
///
/// 1. [`resolve`] no longer accepts `mole`, so the legacy entrypoint is not reachable by name.
/// 2. **A preview states `--dry-run` on the wire rather than inheriting it.** This is the part
///    that does not depend on resolution being right. `--dry-run` reads as a preview under BOTH
///    conventions — the engine accepts-and-ignores it (it is already the default, and
///    `wants_apply` reads it as beating `--apply`), bash maps it to `MOLE_DRY_RUN=1` — so a
///    preview is a preview whatever answered. And `--apply` exists in NEITHER bash flag parser:
///    every one of them ends in a `-*)` arm that prints `Unknown … option:` and exits 1. So the
///    live run either runs on the engine or is REFUSED by a legacy binary. There is no argv this
///    function can emit that deletes something the caller only asked to preview.
///
/// The two appends are the arms of one `if`, mutually exclusive by control flow, so this can
/// never produce the `--apply --dry-run` pair the engine rejects.
///
/// Read-only commands (status/analyze/history) carry neither flag and always run.
pub fn plan(command: &str, user_args: &[String]) -> Result<Plan, String> {
    let spec = engine_spec(command)?;
    let apply = user_args.iter().any(|a| a == "--apply");
    let stream = user_args.iter().any(|a| a == "--stream");
    // Whether `plan` re-emits `--apply` itself for this command, and must therefore filter the
    // user's copy out first rather than sending it twice.
    let strip_apply = matches!(spec.writes, Writes::ApplyOrDryRun | Writes::ApplyOnly);
    let passthrough: Vec<String> = user_args
        .iter()
        .filter(|a| {
            !(is_conductor_flag(a)
                || (strip_apply && *a == "--apply")
                || (*a == "--stream" && spec.engine_streams.contains(&"--stream")))
        })
        .cloned()
        .collect();

    match spec.writes {
        // Read-only. The arguments are the engine's own (`--limit`, `--installed`,
        // `--threshold`, `--app`, `--keep`, positional paths) and pass straight through —
        // including a stray `--apply`, which the engine refuses.
        Writes::Never => {
            let mut args = vec![command.to_string()];
            args.extend(passthrough);
            Ok(Plan { args })
        }
        Writes::ApplyOnly => {
            let mut args = vec![command.to_string()];
            args.extend(passthrough);
            if apply {
                args.push("--apply".into());
            }
            Ok(Plan { args })
        }
        Writes::ApplyOrDryRun if command == "uninstall" => {
            // `--list` short-circuits inside the engine before any destructive code and takes no
            // app name (`cli.rs:745`), so it is neither gated on a positional nor given an
            // apply/dry-run flag — there is nothing for one to qualify.
            if passthrough.iter().any(|a| a == "--list") {
                // ...and a caller who asked to APPLY is told so, rather than handed a listing.
                //
                // `--apply` has already been removed from `passthrough` by the time control
                // reaches here, so the untouched alternative is not "forward it" — it is "drop
                // it", which is what this used to do: a caller who asked for a write got a read
                // and an `ok:true` envelope, with the request gone and nothing saying so.
                // Re-emitting instead would only move the same silent partial one process away,
                // since `--list` short-circuits in the engine BEFORE anything a `--apply` could
                // act on; the caller would still get a listing and a success.
                //
                // So it is refused here. That is the repo's stance (a refusal beats a silent
                // partial) and it is also the only answer that needs no assumption about how the
                // engine treats a pair the conductor is the one mangling.
                if apply {
                    return Err(
                        "uninstall: --list only enumerates installed apps and removes nothing, so \
                         --apply has nothing to qualify. Drop --apply, or name an app to uninstall."
                            .into(),
                    );
                }
                let mut args = vec!["uninstall".to_string()];
                args.extend(passthrough);
                return Ok(Plan { args });
            }
            if passthrough.iter().all(|a| a.starts_with('-')) {
                return Err("uninstall: needs at least one app name (or --list)".into());
            }
            Ok(destructive(spec, apply, stream, &passthrough))
        }
        Writes::ApplyOrDryRun => Ok(destructive(spec, apply, stream, &passthrough)),
        // Pinned impossible by `commands::tests::write_conventions_match_their_owner`.
        Writes::Native => Err(format!("{command} is not an engine command")),
    }
}

/// One destructive command's argv: the subcommand, the user's own arguments, the streaming
/// transport where the engine implements it, and exactly one of `--apply` / `--dry-run`.
fn destructive(spec: &Spec, apply: bool, stream: bool, extra: &[String]) -> Plan {
    let mut args = vec![spec.name.to_string()];
    args.extend(extra.iter().cloned());
    if stream && spec.engine_streams.contains(&"--stream") {
        args.push("--stream".into());
    }
    // Exactly one of the pair, always stated. See `plan`'s doc for why the preview does not
    // simply inherit the engine's default.
    args.push(if apply { "--apply" } else { "--dry-run" }.into());
    Plan { args }
}

/// Whether this invocation should be forwarded with the engine's stdio inherited, so its NDJSON
/// flows line-by-line instead of arriving as one buffered envelope.
///
/// True only for a streaming flag on a command whose table row lists it (`engine_streams`):
/// `clean --stream`, `optimize --stream`, `purge --stream`, `status --watch` and
/// `analyze --progress`. It is deliberately NOT true for "stdout is a TTY", which is what used to
/// select this path: the digger rendered a colored TUI worth showing, and the engine has exactly
/// one output mode. Inheriting stdio for a non-streaming command would print the ENGINE's
/// envelope — carrying the engine's version in `burrow_cli` and bypassing `--raw` — where
/// capturing it lets the conductor emit its own.
///
/// Inherited stdio is also what makes the stream LIVE when piped: the engine writes each frame
/// straight to the caller's pipe as it happens, and there is no conductor buffer in between to
/// hold a `status --watch` tick back until the watch ends.
pub fn is_stream_passthrough(command: &str, user_args: &[String]) -> bool {
    engine_spec(command).is_ok_and(|spec| {
        spec.engine_streams
            .iter()
            .any(|flag| user_args.iter().any(|a| a == flag))
    })
}

/// The engine binary's name. Deliberately does NOT include `mole`: see [`plan`].
const ENGINE_BIN: &str = "burrow-engine";

/// Resolve the `burrow-engine` executable.
///
/// In order: `$BURROW_ENGINE` (a path to the binary, or a directory containing it — same shape
/// as `$BURROW_FCLONES`/`$BURROW_CZKAWKA`/`$BURROW_BCU`), then `$BURROW_ENGINE_DIR` (the older
/// spelling, kept working), then beside the `burrow` executable itself (how a bundled sidecar
/// ships), then a sibling checkout discovered by walking up, then `PATH`.
///
/// Only the walk-up probes a checkout at its root AND at `target/release` + `target/debug`,
/// because a Rust source checkout keeps the binary under `target/` — the digger kept its
/// entrypoint at the directory root, so the old walk-up only ever looked there. The two
/// environment overrides are ROOT-ONLY: `$BURROW_ENGINE`/`$BURROW_ENGINE_DIR` pointing at a
/// directory must contain the binary directly (`resolve_env_executable` → `resolve_in_dir`), and
/// a checkout named there resolves nothing.
///
/// `mole` is not a candidate at any step. It was, on the theory that a legacy entrypoint is
/// better than none; with the argv conventions inverted, a legacy entrypoint driven by this
/// module's plans is worse than none.
pub fn resolve() -> Result<PathBuf, Failure> {
    for var in ["BURROW_ENGINE", "BURROW_ENGINE_DIR"] {
        match platform::resolve_env_executable(var, &[ENGINE_BIN]) {
            Ok(Some(p)) => return Ok(p),
            // A set-but-wrong override is an error, not a reason to search elsewhere: silently
            // falling through to a different engine than the one named is how you debug the
            // wrong binary for an hour. It stays `not_found` — the kind a GUI branches on to say
            // "the engine isn't installed" rather than showing a generic failure.
            Err(e) => {
                return Err(Failure::not_found(format!(
                    "could not locate {ENGINE_BIN}: {}",
                    e.message
                )))
            }
            Ok(None) => {}
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        // Beside the conductor (bundled sidecar).
        if let Some(dir) = exe.parent() {
            if let Some(found) = platform::resolve_in_dir(dir, &[ENGINE_BIN]) {
                return Ok(found);
            }
        }
        // A sibling checkout, walking up.
        let mut dir = exe.parent().map(Path::to_path_buf);
        while let Some(d) = dir {
            let checkout = d.join(ENGINE_BIN);
            for sub in ["", "target/release", "target/debug"] {
                let probe = if sub.is_empty() {
                    checkout.clone()
                } else {
                    checkout.join(sub)
                };
                if let Some(found) = platform::resolve_in_dir(&probe, &[ENGINE_BIN]) {
                    return Ok(found);
                }
            }
            dir = d.parent().map(Path::to_path_buf);
        }
    }
    if let Some(found) = platform::resolve_on_path(&[ENGINE_BIN]) {
        return Ok(found);
    }
    Err(Failure::not_found(format!(
        "could not locate {ENGINE_BIN}; set BURROW_ENGINE=/path/to/{ENGINE_BIN}"
    )))
}

/// Run a plan with the terminal inherited — the engine's NDJSON stream reaches the caller
/// line-by-line. Returns the engine's exit code.
pub fn execute_native(bin: &Path, plan: &Plan) -> Result<i32, Failure> {
    let status = platform::command(bin, &plan.args)?
        .status()
        .map_err(|e| Failure::io(format!("failed to run {}", bin.display()), &e))?;
    Ok(status.code().unwrap_or(1))
}

/// Execute a plan and return the engine's PAYLOAD — the `data` subtree of its envelope, with the
/// envelope itself removed. See [`payload_of`] for why the conductor unwraps rather than
/// forwarding or nesting.
///
/// The error side carries a [`Failure`] rather than a bare string so a refusal the engine already
/// classified reaches the envelope with that classification intact. Two of the three ways this
/// can fail are the CONDUCTOR's own — a spawn that never started, an envelope it could not read —
/// and those say so, because there is nothing to relay.
pub fn execute(bin: &Path, plan: &Plan) -> Result<String, Failure> {
    let out = platform::command(bin, &plan.args)?
        .output()
        .map_err(|e| Failure::io(format!("failed to run {}", bin.display()), &e))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let failed_envelope = serde_json::from_str::<serde_json::Value>(stdout.trim())
        .ok()
        .is_some_and(|value| value.get("ok").and_then(serde_json::Value::as_bool) == Some(false));
    if !out.status.success() || failed_envelope {
        // The engine writes EVERYTHING to stdout, including its classified failure envelope, and
        // leaves stderr empty (`burrow-engine/src/main.rs` is a single `println!`). Reading
        // stderr here — which is what this did — produced `engine … exited exit status: 2:` with
        // the reason cut off, for every refusal the engine makes: an unknown flag, `--apply`
        // with `--dry-run`, a missing HOME. So the message comes off the envelope.
        return Err(engine_failure(bin, &out.status.to_string(), &stdout));
    }
    payload_of(&stdout).map_err(Failure::invalid_output)
}

/// The failure an engine run reported, read off the engine's own envelope: its `error.message`
/// AND the `error.kind` it had already classified that message as.
///
/// # Relaying the kind is all-or-nothing, and so is falling back
///
/// The kind is taken only when the message was — they are one classified answer, and the kind
/// describes THAT message. An engine envelope carrying a kind but no readable message leaves the
/// conductor writing its own text ("engine … exited 2"), and stamping the engine's
/// classification onto text the engine did not write attributes a judgement to the wrong prose.
/// So that case falls back completely to `process_failed`, which is the one thing the conductor
/// can actually stand behind: a process ran and exited non-zero.
///
/// A missing `kind`, or one that is not a JSON string, or an empty one, is treated as absent for
/// the same reason: none of them is a classification, and inventing one would be worse than
/// admitting the conductor does not know — the relayed message is carried with the generic
/// `error`. An unrecognized *non-empty* kind is different: that is a classification, just not one
/// this conductor has heard of, and it is relayed verbatim as `ErrorKind::Other`.
///
/// The engine's top-level `feature` key is deliberately not read here. `README.md` documents its
/// presence as how a caller tells a conductor-side refusal from a relayed engine one, and the
/// conductor's own `unsupported_envelope` is the only thing that emits it.
fn engine_failure(bin: &Path, status: &str, stdout: &str) -> Failure {
    let error = serde_json::from_str::<serde_json::Value>(stdout.trim())
        .ok()
        .and_then(|v| v.get("error").cloned());
    if let Some(message) = error
        .as_ref()
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
    {
        let kind = error
            .as_ref()
            .and_then(|e| e.get("kind"))
            .and_then(|k| k.as_str())
            .filter(|k| !k.is_empty());
        let failure = match kind {
            Some(kind) => Failure::relayed(message, kind),
            None => Failure::error(message),
        };
        return match error.as_ref().and_then(|error| error.get("details")) {
            Some(details) => failure.with_details(details.clone()),
            None => failure,
        };
    }
    let trimmed = stdout.trim();
    Failure::process_failed(if trimmed.is_empty() {
        format!("engine {} exited {status}", bin.display())
    } else {
        format!("engine {} exited {status}: {trimmed}", bin.display())
    })
}

/// Strip the engine's envelope and return the payload it wrapped.
///
/// # Why unwrap, rather than forward the envelope or nest it
///
/// The engine emits the same `{ok, burrow_cli, engine, command, data}` shape the conductor does
/// — its `envelope.rs` was ported verbatim from `output.rs` — so all three are mechanically
/// possible. What decides it is that three consumers already read the CURRENT shape:
///
/// - **The conductor's own commands.** `snapshot`, `watch` and `trigger` run `status` through
///   [`execute`] and hand the result to `metrics::sample_from_status` /
///   `alerts_from_status`, which read `health_score`, `cpu`, `disks` and `process_alerts` at the
///   TOP LEVEL. Forwarding the envelope makes every one of those lookups miss and silently
///   return the `unwrap_or` default — a health score of -1 and 0% disk recorded into history, no
///   error anywhere. This is the decisive one: it breaks inside this binary, quietly.
/// - **The macOS app.** `BurrowEnvelope.parse` takes `dict["data"]`, re-serializes it, and hands
///   the bytes to the command's concrete decoder (`MoleStatus`, `DiskScanner.parse`). Nesting
///   would put an envelope where a `MoleStatus` is expected and fail the decode.
/// - **`burrow_cli` names the CONDUCTOR.** Forwarding verbatim would report the engine's version
///   there (`0.1.0`) rather than the conductor's — a field whose whole job is to say which
///   conductor answered, filled in by something else. The engine's own `version` doc warns about
///   consumers scraping a version out of this envelope; handing them the wrong one is worse.
///
/// So the conductor unwraps and re-wraps, and the bytes on stdout keep the shape every consumer
/// already reads, with the version field telling the truth.
///
/// **`uninstall --list` is the one command that does not emit an envelope at all** — it prints a
/// bare JSON array (`cli.rs:1598-1603`). Valid bare JSON is returned as the payload, while
/// malformed JSON is a protocol failure.
pub fn payload_of(stdout: &str) -> Result<String, String> {
    let trimmed = stdout.trim();
    let v = serde_json::from_str::<serde_json::Value>(trimmed)
        .map_err(|error| format!("invalid engine output: {error}"))?;
    // An envelope is an object carrying `ok` — the field every consumer branches on.
    let is_envelope = v.get("ok").and_then(serde_json::Value::as_bool).is_some();
    if !is_envelope {
        return Ok(trimmed.to_string());
    }
    match v.get("data") {
        Some(data) => Ok(data.to_string()),
        // An `ok:true` envelope with no `data` is a shape this conductor cannot forward
        // meaningfully; say so rather than emitting `null` as if it were a result.
        None => Err(format!(
            "invalid engine output: success envelope carried no `data` ({trimmed})"
        )),
    }
}

#[cfg(all(test, windows))]
fn touch(path: &Path) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, "").unwrap();
}

#[cfg(all(test, windows))]
fn temp_engine_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("burrow_engine_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::ErrorKind;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// The engine commands whose table row spells writes as `w`.
    fn engine_commands_with(w: Writes) -> Vec<&'static str> {
        commands::COMMANDS
            .iter()
            .filter(|s| s.owner == Owner::Engine && s.writes == w)
            .map(|s| s.name)
            .collect()
    }

    /// The commands the engine ALSO implements, which burrow-cli used to implement itself until
    /// #18. Kept as a test-local list because it is history, not routing: the table has one
    /// `Owner::Engine` and no notion of "moved".
    const MOVED_IN_18: &[&str] = &[
        "net",
        "orphans",
        "evict",
        "dupes",
        "slim-check",
        "sentinel",
        "photos",
        "rules",
    ];

    #[test]
    fn status_forwards_bare() {
        // No `--json`: the engine has one output mode and GLOBAL_FLAGS accepting the flag does
        // not make sending it meaningful.
        assert_eq!(plan("status", &[]).unwrap().args, a(&["status"]));
    }

    #[test]
    fn analyze_passes_positional_path() {
        assert_eq!(
            plan("analyze", &a(&["/tmp"])).unwrap().args,
            a(&["analyze", "/tmp"])
        );
    }

    #[test]
    fn history_forwards_limit() {
        assert_eq!(
            plan("history", &a(&["--limit", "5"])).unwrap().args,
            a(&["history", "--limit", "5"])
        );
    }

    /// The inversion, in the direction that destroys data: a default `clean` must be a PREVIEW,
    /// and it must say so on the wire rather than inheriting a default. See `plan`'s doc.
    #[test]
    fn destructive_commands_default_to_an_explicit_dry_run() {
        let destructive = engine_commands_with(Writes::ApplyOrDryRun);
        assert_eq!(
            destructive,
            ["clean", "optimize", "purge", "uninstall", "installer"],
            "the engine's `allowed_flags` declares --apply AND --dry-run on exactly these"
        );
        for cmd in destructive {
            let p = plan(cmd, &[]).unwrap_or_else(|_| plan(cmd, &a(&["Foo.app"])).unwrap());
            assert!(
                p.args.contains(&"--dry-run".to_string()),
                "{cmd} preview must state --dry-run, got {:?}",
                p.args
            );
            assert!(
                !p.args.contains(&"--apply".to_string()),
                "{cmd} preview must not carry --apply, got {:?}",
                p.args
            );
        }
    }

    #[test]
    fn apply_replaces_the_dry_run_guard_and_never_joins_it() {
        for cmd in engine_commands_with(Writes::ApplyOrDryRun) {
            let args = a(&["--apply"]);
            let p = plan(cmd, &args)
                .unwrap_or_else(|_| plan(cmd, &a(&["Foo.app", "--apply"])).unwrap());
            assert!(
                p.args.contains(&"--apply".to_string()),
                "{cmd} --apply must reach the engine, got {:?}",
                p.args
            );
            // The engine refuses the pair outright (exit 2); the two appends are one `if`.
            assert!(
                !p.args.contains(&"--dry-run".to_string()),
                "{cmd} must never send both, got {:?}",
                p.args
            );
        }
    }

    #[test]
    fn clean_preview_and_apply_are_exact() {
        assert_eq!(plan("clean", &[]).unwrap().args, a(&["clean", "--dry-run"]));
        assert_eq!(
            plan("clean", &a(&["--apply"])).unwrap().args,
            a(&["clean", "--apply"])
        );
    }

    #[test]
    fn purge_forwards_extra_flags_before_the_guard() {
        // `--permanent` is a real engine flag on purge (opt out of Trash routing). It used to be
        // `--include-empty` here, which the engine REFUSES on purpose — it widens the scan and is
        // not implemented, so accepting it would report a narrower result as the wider one.
        let p = plan("purge", &a(&["--permanent"])).unwrap();
        assert_eq!(p.args, a(&["purge", "--permanent", "--dry-run"]));
    }

    #[test]
    fn uninstall_requires_an_app_and_defaults_to_dry_run() {
        assert!(plan("uninstall", &[]).is_err());
        // A flag alone is not an app name — the old check was `passthrough.is_empty()`, which
        // let `uninstall --permanent` through as if `--permanent` were the app.
        assert!(plan("uninstall", &a(&["--permanent"])).is_err());
        assert_eq!(
            plan("uninstall", &a(&["Foo.app"])).unwrap().args,
            a(&["uninstall", "Foo.app", "--dry-run"])
        );
    }

    #[test]
    fn uninstall_list_needs_no_app_and_carries_no_guard() {
        // `--list` short-circuits in the engine before any destructive code; an apply/dry-run
        // flag would qualify nothing.
        let p = plan("uninstall", &a(&["--list"])).unwrap();
        assert_eq!(p.args, a(&["uninstall", "--list"]));
    }

    /// A caller asking to LIST and to APPLY is refused, not quietly given the listing.
    ///
    /// `strip_apply_for("uninstall")` filters `--apply` out of the passthrough, so the `--list`
    /// short-circuit used to emit a plain listing and an `ok:true` envelope with the write request
    /// dropped and nothing to notice. There is no argv that satisfies both halves — `--list`
    /// returns in the engine before anything an `--apply` could act on — so the pair is answered
    /// rather than half-served.
    #[test]
    fn uninstall_list_with_apply_is_refused_rather_than_silently_listed() {
        let e = plan("uninstall", &a(&["--list", "--apply"])).unwrap_err();
        assert!(e.contains("--apply"), "the refusal must name the flag: {e}");
        // Order must not change the answer: the pair is the pair.
        assert!(plan("uninstall", &a(&["--apply", "--list"])).is_err());
        // And an app name alongside is still the pair — `--list` takes no app, so this is not a
        // legitimate `uninstall Foo.app --apply` that happens to carry a stray flag.
        assert!(plan("uninstall", &a(&["Foo.app", "--list", "--apply"])).is_err());
    }

    #[test]
    fn raw_and_json_never_reach_the_engine() {
        // `--raw` is REFUSED by the engine (exit 2), so forwarding it would turn every `--raw`
        // invocation into a hard failure; `--json` is a no-op there.
        let p = plan("status", &a(&["--raw", "--json"])).unwrap();
        assert_eq!(p.args, a(&["status"]));
    }

    #[test]
    fn stream_reaches_only_the_commands_that_implement_it() {
        assert_eq!(
            plan("clean", &a(&["--stream"])).unwrap().args,
            a(&["clean", "--stream", "--dry-run"])
        );
        assert_eq!(
            plan("optimize", &a(&["--stream", "--apply"])).unwrap().args,
            a(&["optimize", "--stream", "--apply"])
        );
        assert_eq!(
            plan("purge", &a(&["--stream"])).unwrap().args,
            a(&["purge", "--stream", "--dry-run"])
        );
        assert_eq!(
            plan("purge", &a(&["--stream", "--apply"])).unwrap().args,
            a(&["purge", "--stream", "--apply"])
        );
        // installer/uninstall don't stream — the engine refuses the flag there.
        assert_eq!(
            plan("installer", &a(&["--stream"])).unwrap().args,
            a(&["installer", "--stream", "--dry-run"])
        );
        assert_eq!(
            plan("status", &a(&["--stream"])).unwrap().args,
            a(&["status", "--stream"])
        );
    }

    /// `status --watch [--interval <secs>]` and `analyze --progress <path>` are read-only rows,
    /// so their flags are the engine's own and pass straight through — `--interval` and its
    /// value included, in the caller's order. Nothing is added: the engine's watch has no
    /// apply/dry-run half to state.
    #[test]
    fn watch_and_progress_pass_through_verbatim() {
        assert_eq!(
            plan("status", &a(&["--watch"])).unwrap().args,
            a(&["status", "--watch"])
        );
        assert_eq!(
            plan("status", &a(&["--watch", "--interval", "0.5", "--raw"]))
                .unwrap()
                .args,
            a(&["status", "--watch", "--interval", "0.5"])
        );
        assert_eq!(
            plan("analyze", &a(&["--progress", "/x"])).unwrap().args,
            a(&["analyze", "--progress", "/x"])
        );
    }

    /// `clean --apply --plan <file>` executes exactly a held preview. `--plan` and its value are
    /// the engine's and are forwarded untouched, with the conductor's apply/dry-run translation
    /// still stated after them.
    #[test]
    fn clean_plan_is_forwarded_with_the_apply_translation() {
        assert_eq!(
            plan("clean", &a(&["--apply", "--plan", "/x/plan.json"]))
                .unwrap()
                .args,
            a(&["clean", "--plan", "/x/plan.json", "--apply"])
        );
        assert_eq!(
            plan("clean", &a(&["--plan", "/x/plan.json"])).unwrap().args,
            a(&["clean", "--plan", "/x/plan.json", "--dry-run"])
        );
    }

    #[test]
    fn stream_passthrough_is_only_for_streaming_commands() {
        assert!(is_stream_passthrough("clean", &a(&["--stream"])));
        assert!(is_stream_passthrough("optimize", &a(&["--stream"])));
        assert!(is_stream_passthrough("purge", &a(&["--stream"])));
        assert!(is_stream_passthrough("purge", &a(&["--stream", "--apply"])));
        assert!(is_stream_passthrough("status", &a(&["--watch"])));
        assert!(is_stream_passthrough(
            "status",
            &a(&["--watch", "--interval", "5"])
        ));
        assert!(is_stream_passthrough("analyze", &a(&["--progress", "/x"])));
        // The wrong stream flag for the command is not a stream: it is forwarded, refused by the
        // engine with a buffered envelope, and that refusal is relayed — not printed raw.
        assert!(!is_stream_passthrough("installer", &a(&["--stream"])));
        assert!(!is_stream_passthrough("status", &a(&["--stream"])));
        assert!(!is_stream_passthrough("analyze", &a(&["--watch"])));
        assert!(!is_stream_passthrough("clean", &a(&["--progress"])));
        // `--interval` qualifies a watch; alone it is an ordinary (refused) engine flag.
        assert!(!is_stream_passthrough("status", &a(&["--interval", "5"])));
        // `--plan` is an argument, not a transport.
        assert!(!is_stream_passthrough(
            "clean",
            &a(&["--apply", "--plan", "/x/plan.json"])
        ));
        // A TTY is not a reason to inherit stdio any more — the engine has no TUI to render.
        assert!(!is_stream_passthrough("clean", &[]));
    }

    #[test]
    fn unknown_command_errors() {
        assert!(plan("bogus", &[]).is_err());
    }

    #[test]
    fn overlap_commands_forward_their_engine_arguments_verbatim() {
        assert_eq!(
            plan("photos", &a(&["/tmp", "--threshold", "8"]))
                .unwrap()
                .args,
            a(&["photos", "/tmp", "--threshold", "8"])
        );
        assert_eq!(
            plan("rules", &a(&["dryrun", "rules", "--app", "com.x.y"]))
                .unwrap()
                .args,
            a(&["rules", "dryrun", "rules", "--app", "com.x.y"])
        );
        assert_eq!(
            plan("net", &a(&["--limit", "3"])).unwrap().args,
            a(&["net", "--limit", "3"])
        );
        assert_eq!(
            plan("orphans", &a(&["/tmp", "--installed", "a,b"]))
                .unwrap()
                .args,
            a(&["orphans", "/tmp", "--installed", "a,b"])
        );
        assert_eq!(
            plan("dupes", &a(&["group", "/tmp", "--keep", "/k"]))
                .unwrap()
                .args,
            a(&["dupes", "group", "/tmp", "--keep", "/k"])
        );
    }

    /// `evict` and `dupes` take `--apply` but no `--dry-run`, so they get the only translation
    /// available: present or absent. Nothing else may be appended — a `--dry-run` would be an
    /// unknown flag there, refused with exit 2.
    #[test]
    fn apply_only_commands_get_apply_and_never_a_dry_run() {
        let apply_only = engine_commands_with(Writes::ApplyOnly);
        assert_eq!(apply_only, ["dupes", "evict"]);
        for cmd in apply_only {
            let preview = plan(cmd, &a(&["/tmp"])).unwrap();
            assert_eq!(preview.args, a(&[cmd, "/tmp"]), "{cmd} preview");
            let live = plan(cmd, &a(&["/tmp", "--apply"])).unwrap();
            assert_eq!(live.args, a(&[cmd, "/tmp", "--apply"]), "{cmd} apply");
        }
    }

    /// `--apply` requests a WRITE, so on a command that cannot write it must reach the engine and
    /// be refused there — not be swallowed here, which would tell the caller the write was
    /// honoured. Contrast the output/transport flags, which the conductor legitimately owns.
    #[test]
    fn apply_is_not_swallowed_on_commands_that_cannot_write() {
        for cmd in [
            "photos",
            "net",
            "orphans",
            "slim-check",
            "rules",
            "sentinel",
        ] {
            let p = plan(cmd, &a(&["--apply"])).unwrap();
            assert!(
                p.args.contains(&"--apply".to_string()),
                "{cmd} must forward --apply so the engine refuses it, got {:?}",
                p.args
            );
        }
    }

    /// Routing no longer varies by platform, and since `burrow-engine 5e42bf0` nothing else does
    /// either. It used to vary twice over: the overlapping commands had a native implementation
    /// here, so `owns` answered "the engine's on macOS, mine on Windows"; and after those were
    /// deleted a `windows_refusal` table still pre-empted two of them before dispatch. Both are
    /// gone, so this is now the ONLY platform-shaped question the conductor asks about these
    /// eight, and it has one answer everywhere.
    #[test]
    fn every_command_the_engine_serves_is_the_engines_on_every_platform() {
        for cmd in ["status", "analyze", "history", "clean", "uninstall"] {
            assert!(owns(cmd), "{cmd} has only ever been the engine's");
        }
        for cmd in MOVED_IN_18 {
            assert!(
                owns(cmd),
                "{cmd} moved to the engine on every platform, not just macOS"
            );
        }
        // ...and the table agrees with `owns` row for row, because `owns` IS the table.
        for spec in commands::COMMANDS {
            assert_eq!(
                owns(spec.name),
                spec.owner == Owner::Engine,
                "{}",
                spec.name
            );
        }
        // The conductor's remaining surface: no engine implementation exists for any of these,
        // so `owns` must keep saying no or `run` would forward a command the engine refuses.
        for cmd in ["trash", "win-dupes", "win-uninstall", "slim", "diff", "gui"] {
            assert!(!owns(cmd), "{cmd} is the conductor's own");
        }
    }

    /// Every overlapping command is PLANNED identically on every platform, which is the local
    /// half of "the engine owns the platform answer".
    ///
    /// This is what the retired `windows_refusal` table used to make false for two of them: a
    /// plan was produced and then thrown away unspawned. The end-to-end half — that the engine is
    /// actually reached, and that what it says is what the caller gets — is
    /// `the_two_commands_that_used_to_be_pre_empted_now_reach_the_engine` in `tests/cli.rs`,
    /// which needs a real engine (or a replayed capture of one) and so cannot live here.
    #[test]
    fn no_overlapping_command_is_planned_differently_because_of_the_platform() {
        for cmd in MOVED_IN_18 {
            let p = plan(cmd, &a(&["/tmp/x"])).unwrap_or_else(|e| panic!("{cmd}: {e}"));
            assert_eq!(
                p.args[0], **cmd,
                "{cmd} must plan as itself, unconditionally: {:?}",
                p.args
            );
            assert!(
                p.args.contains(&"/tmp/x".to_string()),
                "{cmd} must forward the caller's argument rather than short-circuit: {:?}",
                p.args
            );
        }
    }

    #[test]
    fn payload_is_the_data_subtree() {
        let env = r#"{"ok":true,"burrow_cli":"0.1.0","engine":"burrow-engine","command":"status","data":{"health_score":91}}"#;
        assert_eq!(payload_of(env).unwrap(), r#"{"health_score":91}"#);
    }

    /// `uninstall --list` is the one command that emits no envelope — a bare JSON array. It has
    /// nothing to strip, so it passes through as the payload.
    #[test]
    fn a_bare_array_is_already_the_payload() {
        assert_eq!(
            payload_of(r#"[{"name":"IDLE"}]"#).unwrap(),
            r#"[{"name":"IDLE"}]"#
        );
    }

    #[test]
    fn non_json_engine_output_is_a_protocol_failure() {
        for output in ["not json", "", "{broken", "[{\"unfinished\":true}"] {
            assert!(payload_of(output).is_err(), "{output}");
        }
    }

    #[test]
    fn a_success_envelope_without_data_is_an_error_not_a_null() {
        let env = r#"{"ok":true,"burrow_cli":"0.1.0","command":"status"}"#;
        let e = payload_of(env).unwrap_err();
        assert!(e.contains("invalid engine output"), "got: {e}");
    }

    /// Every REAL engine failure envelope captured under `testdata/engine-captures/`, which
    /// between them cover all three classifications the engine emits — `unsupported` (two,
    /// provoked with `sandbox-exec`, plus the reworded build), `error` (two), and `not_found`.
    /// See that directory's `PROVENANCE.txt` for how each was produced.
    ///
    /// `slim-check-error.json` is not an oddity and needs no special handling: the ENGINE put
    /// `"kind":"error"` in that envelope, and the conductor relays it like any other. It reads
    /// like a conductor fall-through only because `error` is also the conductor's generic kind,
    /// so on this message relaying and guessing would have landed on the same answer — which is
    /// exactly why the reworded capture had to exist to tell the two apart. Kept here as the
    /// proof the relay reports `error` when `error` is what was said, rather than only ever
    /// upgrading a failure to `unsupported`.
    const CAPTURES: &[&str] = &[
        include_str!("../testdata/engine-captures/net-unsupported.json"),
        include_str!("../testdata/engine-captures/orphans-unsupported.json"),
        include_str!("../testdata/engine-captures/slim-check-error.json"),
        include_str!("../testdata/engine-captures/net-unsupported-reworded.json"),
        include_str!("../testdata/engine-captures/evict-error.json"),
        include_str!("../testdata/engine-captures/sentinel-not-found.json"),
    ];

    /// The engine puts its classified failure on STDOUT and leaves stderr empty, so both the
    /// reason AND the classification have to be read off the envelope or they are lost.
    ///
    /// Driven off the captures rather than a typed-out envelope: each one's `error.message` and
    /// `error.kind` are read from the capture itself and compared with what `engine_failure`
    /// lifted, so the test cannot drift into asserting a shape nobody emits.
    #[test]
    fn a_failure_relays_the_engines_own_reason_and_classification() {
        for capture in CAPTURES {
            let v: serde_json::Value = serde_json::from_str(capture).expect("capture is JSON");
            let failure = engine_failure(Path::new("/x/burrow-engine"), "exit status: 1", capture);
            assert_eq!(
                failure.message,
                v["error"]["message"].as_str().unwrap(),
                "message must come off the envelope: {capture}"
            );
            assert_eq!(
                failure.kind().as_str(),
                v["error"]["kind"].as_str().unwrap(),
                "the engine's classification must be relayed, not re-derived: {capture}"
            );
        }
    }

    #[test]
    fn a_failure_with_no_envelope_still_names_the_binary_and_status() {
        let f = engine_failure(Path::new("/x/burrow-engine"), "exit status: 101", "");
        assert!(f.message.contains("/x/burrow-engine"), "got: {}", f.message);
        assert!(f.message.contains("101"), "got: {}", f.message);
        assert_eq!(
            f.kind(),
            &ErrorKind::ProcessFailed,
            "nothing was relayed; a non-zero exit with no envelope is a process failure"
        );
    }

    /// An envelope with a message but no `kind` is the honest fallback case: relay the reason
    /// under the generic `error`, since nothing classified it and the conductor will not guess.
    /// Built by DELETING `kind` from a real capture, so the rest of the shape is still the
    /// engine's.
    #[test]
    fn an_engine_failure_without_a_kind_is_relayed_as_the_generic_error() {
        let mut v: serde_json::Value = serde_json::from_str(CAPTURES[0]).unwrap();
        let message = v["error"]["message"].as_str().unwrap().to_string();
        v["error"].as_object_mut().unwrap().remove("kind");

        let f = engine_failure(
            Path::new("/x/burrow-engine"),
            "exit status: 1",
            &v.to_string(),
        );
        assert_eq!(f.message, message);
        assert_eq!(f.kind(), &ErrorKind::Error, "no kind to relay");
    }

    /// A `kind` that is not a non-empty STRING is not a classification. Relaying `""` or a number
    /// would put a value in `error.kind` that no consumer can branch on, so both are treated as
    /// absent and the generic `error` answers instead.
    #[test]
    fn a_kind_that_is_not_a_usable_string_is_treated_as_absent() {
        for bad in [
            serde_json::json!(""),
            serde_json::json!(7),
            serde_json::json!(null),
        ] {
            let mut v: serde_json::Value = serde_json::from_str(CAPTURES[0]).unwrap();
            v["error"]["kind"] = bad.clone();
            let f = engine_failure(
                Path::new("/x/burrow-engine"),
                "exit status: 1",
                &v.to_string(),
            );
            assert_eq!(
                f.kind(),
                &ErrorKind::Error,
                "kind {bad} must not be relayed"
            );
        }
    }

    /// Relay is all-or-nothing. An envelope carrying a `kind` but no readable message leaves the
    /// conductor writing its own sentence, and the engine's classification does not describe that
    /// sentence — so it is dropped with the message rather than stamped onto it.
    #[test]
    fn a_kind_without_a_message_is_not_relayed_onto_the_conductors_own_words() {
        let mut v: serde_json::Value = serde_json::from_str(CAPTURES[0]).unwrap();
        v["error"].as_object_mut().unwrap().remove("message");

        let f = engine_failure(
            Path::new("/x/burrow-engine"),
            "exit status: 1",
            &v.to_string(),
        );
        assert_eq!(
            f.kind(),
            &ErrorKind::ProcessFailed,
            "nothing to attach the engine's classification to"
        );
        assert!(
            f.message.contains("/x/burrow-engine"),
            "the conductor names itself as the source: {}",
            f.message
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_resolution_orders_exe_cmd_bat_then_extensionless() {
        let dir = temp_engine_dir("orders_extensions");
        touch(&dir.join("burrow-engine.bat"));
        touch(&dir.join("burrow-engine.cmd"));
        touch(&dir.join("burrow-engine.exe"));
        let found = platform::resolve_in_dir(&dir, &[ENGINE_BIN]).unwrap();
        assert_eq!(found.file_name().unwrap(), "burrow-engine.exe");

        std::fs::remove_file(dir.join("burrow-engine.exe")).unwrap();
        std::fs::remove_file(dir.join("burrow-engine.cmd")).unwrap();
        std::fs::remove_file(dir.join("burrow-engine.bat")).unwrap();
        touch(&dir.join("burrow-engine"));
        let found = platform::resolve_in_dir(&dir, &[ENGINE_BIN]).unwrap();
        assert_eq!(found.file_name().unwrap(), "burrow-engine");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A legacy `mole` entrypoint must NOT resolve: driven with this module's plans it would read
    /// the argv with inverted meaning. See `plan`.
    #[cfg(windows)]
    #[test]
    fn windows_resolution_never_accepts_a_legacy_mole() {
        let dir = temp_engine_dir("no_legacy_mole");
        touch(&dir.join("mole.exe"));
        assert!(platform::resolve_in_dir(&dir, &[ENGINE_BIN]).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
