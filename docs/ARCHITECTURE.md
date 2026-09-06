# Burrow CLI — architecture

This document describes the public source snapshot. Upstream attribution is recorded in
`PROVENANCE.md`; the Windows app's older adapter contract lives on the `windows-legacy` branch.

## Process boundaries

```
Humans / scripts -> burrow CLI (this repo) -> burrow-engine (FSL Rust runtime)
macOS GUI / MCP --------------------------> burrow-engine (same runtime, called directly)
Windows app / MCP -> windows-legacy CLI -> bundled MIT Mole PowerShell runtime

burrow CLI native modules: diff, metrics/digest, trigger, trash, sentinel --watch,
                           slim, Windows czkawka/BCU adapters and GUI launcher
burrow-engine sidecar:      fclones (MIT duplicate discovery / APFS dedupe)
```

The conductor owns what has no engine equivalent: `diff`, the metrics/digest history, the
`trigger --rules` auto-clean planner, the explain-before-delete framing, and the agent surface.
`burrow-engine` is caezium's own core under the same licence as this repo, not a third party;
fclones and the Windows sidecars are the genuinely external ones. All of them are invoked as
separate processes and parsed via JSON — the same arm's-length pattern the GUI already uses.

## Windows sidecar contracts

Two commands, `win-dupes` and `win-uninstall`, are conductor-native Windows adapters and were not
affected by the engine migration.

`win-dupes` uses czkawka but does not expose czkawka's version-dependent JSON directly. The
adapter normalizes duplicate and similar-image scans into Burrow groups with stable
`path`/nullable `size` entries, group and file counts, and nullable `redundant_bytes`. Unknown
fields are ignored; malformed or unrecognized non-empty reports fail as `invalid_output`. A
missing executable returns a preview containing the exact argument vector rather than empty
stdout or a misleading empty scan. Czkawka exit code 11 is a completed scan with matches.
`win-dupes --apply` returns `unsupported` on every platform — discovery here is read-only,
czkawka has no fed-back delete, and a write request the command cannot honour is refused rather
than folded into an `ok:true` preview.

`burrow dupes` no longer falls back to czkawka. It forwards to the engine, which drives fclones
(cross-platform: `$BURROW_FCLONES`, else `fclones` on `PATH` with `.exe` handled) or reports
`not_found` naming the install. **The apply guard survived the move, one commit late and in the
right place.** burrow-cli's own `cfg!(target_os)` refusal of `dupes dedupe|remove|link --apply`
died with `src/dupes.rs` in #18 (`8cff30f`), and for that window the argv really did reach fclones.
`burrow-engine 5e42bf0` restored it there in two layers: an argv gate above `resolve_fclones`, so
neither `$BURROW_FCLONES` nor an `fclones.exe` on `PATH` buys the mutation back, and a rail inside
`execute`, which is `pub` and reachable without going through a CLI at all. It is deliberately not
re-added here — this conductor is not the engine's only caller.

BCU follows an explicit plan/execute boundary. Preview emits `list <app>` without resolving or
launching BCU; apply alone emits `uninstall <app> /Q /U /J=<confidence>`. Missing executables,
process failures, and access/UAC/elevation denial return outer `ok:false` envelopes. Their
`error.details` carries the program, arguments, exit code, stdout and stderr plus `engine` and
`applied` — and nothing else, so agents can audit the attempted operation without receiving
unrelated environment data.

## Decisions locked

- **Engine baseline:** `burrow-engine` = the Rust engine, one binary serving every command it
  owns — the eight it has always served (`status` `analyze` `history` `clean` `optimize` `purge`
  `installer` `uninstall`) plus the eight #18 handed it (`net` `orphans` `evict` `dupes`
  `slim-check` `sentinel` `photos` `rules`). That is `engine::owns`, read off the command table in
  `src/commands.rs`, and it is the whole routing decision. The rest of `burrow`'s surface never
  had an engine implementation and is dispatched natively in `main.rs`: `diff`, `snapshot`, `digest`/`report`, `watch`, `trigger`, `slim`,
  `trash`, `gui`, `win-dupes`, `win-uninstall`, and the `--watch` half of `sentinel`. Several of
  those (`snapshot`, `watch`, `trigger`) drive the engine's `status` internally, which is not the
  same as being served by it.
  The engine replaced the bash+Go fork of tw93/Mole (`9daf936`, V1.42.0, last MIT), which now
  lives at `caezium/burrow-digger`. Do not depend on upstream mo (now GPL-3.0).
  **Its argv convention is INVERTED relative to mo's:** `mo clean` runs LIVE and `--dry-run`
  previews; `burrow-engine clean` previews and `--apply` runs. `engine::plan` states one of the
  pair explicitly on every destructive call — see its doc for why inheriting the default is a
  trap here.
- **License:** FSL-1.1-ALv2 for this CLI **and for `burrow-engine`** — same licence, same
  copyright holder (`caezium`), verified against `caezium/burrow-engine`'s `LICENSE.md` on every
  ref including `main`. The engine has never been MIT; that name belonged to the mo fork it
  replaced. Apache-2.0 for the `burrow.rules` format + rule data; proprietary for any
  cloud/fleet tier.
- **Repos:** separate — `burrow-cli` (this), `caezium/burrow-engine` (the Rust engine, FSL-1.1-ALv2),
  `caezium/burrow-digger` (the archived MIT bash+Go mo fork), `caezium/Burrow` (GUI).
- **Conductor language:** **Rust**. Chosen because it's already in the local toolchain
  (Go was absent and the disk was too constrained to install it safely), it matches the
  fclones/czkawka engines, and it cross-compiles to mac+Windows fine. The conductor shells out to
  the engine and parses its JSON, so the two stay independently replaceable — which is what let
  the engine go from bash+Go to Rust without touching a call site here.
- **Windows scope:** engine-backed platform behavior belongs to `burrow-engine`. Its current
  `net` implementation uses native IP Helper TCP/UDP IPv4/IPv6 owner tables, with a
  `netstat -ano` / `tasklist` fallback, to report per-process connection counts. It does not
  report bandwidth: `total` is a count, byte fields are zero, and metric metadata states the
  limitation. Registry orphan inventory and OneDrive/Cloud Files eviction remain unsupported.
  The conductor retains Windows executable resolution, AppData state paths, Recycle Bin support,
  and the czkawka/BCU adapters behind `win-dupes`/`win-uninstall`.
- **Windows refusals are the engine's too, not just Windows capability.** For one commit an
  `engine::windows_refusal` table refused `sentinel` and `evict` before dispatch, because the
  engine answered those two *wrongly* off macOS rather than not at all. `burrow-engine 5e42bf0`
  moved both guards into the engine — and with them the `dupes …--apply` deletion guard that had
  been lost outright — so the table is retired and every platform answer for the eight forwarded
  commands now comes from one process. The conductor keeps exactly one platform refusal for a
  forwarded command, `sentinel --watch` with no directory, and only because `--watch` never
  reaches the engine at all.

## Decisions still open

- **App integration:** macOS calls the Rust engine directly. Windows retains the
  `windows-legacy` CLI until its separate Mole runtime contract is migrated.

## Command surface

`burrow help` is rendered from the table in `src/commands.rs` — the one place that says what
`burrow` accepts, who serves each command (`burrow-engine` | `native` | `czkawka` | `bcu`, which is
also the envelope's `engine` field), how its writes are spelled, and which flags stream. Every
command emits the `{ok,…}` JSON envelope on stdout, terminal or pipe alike; actions default to
dry-run and take `--apply` to execute. `--apply` is never swallowed: a command that cannot write
either forwards the flag to the engine, which refuses it, or refuses it itself (`win-dupes`,
`uninstall --list`).

Native handlers validate all flags and positional counts before doing work. Missing values,
unknown flags, unsupported writes, and `--apply --dry-run` combinations fail with an error
envelope. `--raw` also applies to native commands. `slim --apply --output` requires a new
destination file and successful ad-hoc signing, so input aliases and existing outputs cannot
be overwritten. Failed scans of `diff` or `sentinel --watch` report their I/O failure.

`trigger --rules` accepts only automation it can represent faithfully in a trash plan: safe,
recommended, opted-in delete-to-trash rules with file or whole-target searches. Unknown fields,
truncate/remove actions, glob/walk-files searches, and conditional directory targets are refused
and named in `skipped_rule_files`. Directory timestamps cannot verify the ages of their contents.

Exactly three things skip the envelope, all deliberate:

- **`--raw`** — the bare payload (the engine's `data` subtree) with no envelope around it.
  Consumed by the conductor; the engine refuses the flag.
- **Raw NDJSON passthrough** — one JSON object per line, no envelope, the engine's stdio
  inherited so each line reaches the caller as it happens. The engine's streams are
  `clean --stream`, `optimize --stream`, `purge --stream`, `status --watch` and
  `analyze --progress`, each listed on its table row under `engine_streams`; the conductor's own
  are `sentinel --watch` (the poll-based daemon in `run_sentinel`) and
  `trash <paths...> --apply --stream` (or `--apply-plan <file>`). The last three engine streams are the
  `burrow-engine` BUR-132 contract, and this conductor's side of it (`engine::is_stream_passthrough`,
  `run_engine`) forwards each of them raw — line-by-line as the engine writes it, pipe or
  terminal, exit code relayed as-is — whether the caller asks for a preview or `--apply`.
  `status --watch` takes the engine's `--interval <secs>`; a stream flag on any other row
  (`installer --stream`, `history --watch`, the digger's `--watch-interval`) is forwarded,
  refused by the engine, and that refusal relayed in the conductor's envelope — never dropped
  and silently answered with one snapshot. `clean --apply --plan <file>` (execute exactly a held
  preview) is not a stream: `--plan` and its file pass through like any engine argument.
- **The plain-text meta commands** `help`, `version` and `telemetry`.

There is **no interactive-TTY native mode**. It existed to show the digger's colored TUI and was
removed in #18 with the digger; `burrow-engine` has one output mode, so a terminal and a pipe get
the same bytes.

### Failure envelopes

Every failure is `{ok:false, engine, command, error:{kind, message, platform[, details]}}`.
`kind` is a typed classification declared by whoever raised the failure (`output::ErrorKind`) —
relayed verbatim from the engine's own envelope when the engine failed, never re-derived from
the message text. A refusal the conductor raised itself before any engine ran additionally
carries a top-level `feature`; a relayed engine refusal does not (see `README.md`).
An engine failure remains a failure even if its process exits zero, and its `error.details`
survives forwarding. Malformed engine JSON fails with `invalid_output`.

`trash --apply-plan` accepts a complete successful trash preview envelope, its bare
`would_trash` payload, or an array of paths; every entry is checked before any deletion starts.
Any failed trash operation causes a nonzero exit. A buffered failure keeps all per-path results
in `error.details.trashed`; a stream emits every per-path result and exits nonzero if any failed.

`error.details`, present when a sidecar process ran or was about to: `program`, `args`,
`exit_code`, `stdout`, `stderr`, **plus `engine` and `applied`**, so the details stand on their
own when a caller stores or forwards them without the envelope. That is deliberately all — an
agent can audit the attempted operation without receiving unrelated environment data.
