//! The command table — the ONE place that says what `burrow` accepts, who serves each command,
//! how its writes are spelled, and which flags turn its output into a raw stream.
//!
//! `main::run` dispatches from it, `engine::owns` and `engine::plan` route and translate from it,
//! `main::engine_for` attributes every envelope from it, and `main::help_text` is rendered from
//! it. Before it existed those were five hand-maintained lists that had to agree and did not:
//! `help_text` labelled `snapshot`/`watch`/`trigger` `[engine]` while `engine_for` said `native`,
//! and `run`'s match, `engine::owns`, `OVERLAP`, `DESTRUCTIVE`, `APPLY_ONLY` and `STREAMABLE`
//! each restated a subset of the same facts. A command is now added in exactly one place.

/// Who serves a command — the value of the envelope's `engine` field on EVERY envelope the
/// command can produce, success and failure alike. It names the implementation the command is
/// bound to, which is a routing fact that holds whether or not the binary is installed: a
/// `win-dupes` whose czkawka is missing still reports `czkawka`, exactly as its `ok:true`
/// not-found preview does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// `burrow-engine`, spawned by `engine::execute`.
    Engine,
    /// This conductor's own code.
    Native,
    /// The `czkawka_cli` sidecar behind `win-dupes`.
    Czkawka,
    /// The `BCU-console.exe` sidecar behind `win-uninstall`.
    Bcu,
}

impl Owner {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Engine => "burrow-engine",
            Self::Native => "native",
            Self::Czkawka => "czkawka",
            Self::Bcu => "bcu",
        }
    }
}

/// How a command spells a write — what `engine::plan` does with the caller's `--apply`.
///
/// The engine variants are derived from `allowed_flags` in the engine's `src/cli.rs`: which
/// commands declare `--apply`, and whether they declare `--dry-run` beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Writes {
    /// Cannot write. `--apply` is FORWARDED untouched so the engine refuses it ("names a
    /// mutation the command does not have") — never swallowed, which would tell a caller a write
    /// was requested and honoured when nothing could be written.
    Never,
    /// The engine declares BOTH `--apply` and `--dry-run`, so exactly one of the pair is always
    /// stated on the wire. See `engine::plan` for why a preview does not inherit the default.
    ApplyOrDryRun,
    /// The engine declares `--apply` without a `--dry-run` counterpart, so the only two argv it
    /// can be sent are "with `--apply`" and "without".
    ApplyOnly,
    /// A conductor-native command: its handler in `main.rs` reads `--apply` itself and previews
    /// without it.
    Native,
}

/// Where a command's help line is grouped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    ReadOnly,
    Action,
}

/// The platform a command is served on — a help-text annotation, not an enforcement point. The
/// engine refuses its own commands off their platform and the native handlers refuse theirs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Any,
    MacOs,
    Windows,
}

/// One row of the table.
#[derive(Debug)]
pub struct Spec {
    pub name: &'static str,
    /// Alternate spellings that dispatch to the same handler and emit under `name`.
    pub aliases: &'static [&'static str],
    pub owner: Owner,
    pub writes: Writes,
    /// Flags under which the ENGINE emits raw NDJSON (one line per frame, no envelope) and the
    /// conductor forwards its stdio instead of capturing and re-wrapping it — line-by-line as it
    /// arrives, terminal or pipe alike. The engine REFUSES these flags on any command not listed,
    /// rather than accepting and ignoring them.
    ///
    /// Five today: `clean --stream`, `optimize --stream`, `purge --stream`, `status --watch` and
    /// `analyze --progress` (the last three are the engine's BUR-132 contract). A flag that only
    /// QUALIFIES a stream — `status --watch`'s `--interval <secs>` — is not listed: it is an
    /// ordinary engine argument that passes through with everything else, and on its own (no
    /// `--watch`) the engine refuses it with a buffered envelope the conductor relays. The
    /// conductor's own streams — `sentinel --watch`, the poll loop in `run_sentinel`, and
    /// `trash --apply-plan … --apply --stream` — are handled in their handlers and are not engine
    /// streams.
    pub engine_streams: &'static [&'static str],
    pub platform: Platform,
    pub section: Section,
    /// The usage column of the help line.
    pub usage: &'static str,
    /// The one-line description.
    pub summary: &'static str,
}

impl Spec {
    /// The bracketed attribution on a help line: who serves it, and where.
    pub fn help_tag(&self) -> String {
        let where_ = match self.platform {
            Platform::Any => "",
            Platform::MacOs => ", macOS-only",
            Platform::Windows => ", Windows",
        };
        format!("[{}{where_}]", self.owner.label())
    }
}

use Owner::*;
use Platform::*;
use Section::*;
use Writes::{ApplyOnly, ApplyOrDryRun, Never};

/// Every command `burrow` accepts, in help order. `version`, `help` and `telemetry` are the
/// meta commands handled before dispatch (`main`) and are not rows.
///
/// # The engine rows, and what they used to be
///
/// Eight of the engine's rows — `net` `orphans` `evict` `dupes` `slim-check` `sentinel` `photos`
/// `rules` — were implemented here as well as in the engine until #18 (`3633c19` before the
/// squash), and are the engine's on every platform now. Four of them had real Windows support
/// the engine does not, dropped deliberately with the modules that carried it:
///
/// - `net` — an IP-Helper `GetExtendedTcpTable`/`GetExtendedUdpTable` walk (the only user of the
///   `windows-sys` dependency, now gone) falling back to `netstat -ano` + `tasklist`. `src/net.rs`.
/// - `orphans` — the Windows uninstall registry (`reg query` over the three HKCU/HKLM/WOW6432Node
///   Uninstall roots) plus an AppData/ProgramData/Program Files scan. `src/orphan.rs`.
/// - `evict` — provider-aware OneDrive dehydration: `%OneDrive%`-family roots, cloud attributes
///   from `MetadataExt::file_attributes()`, eviction via `attrib +U -P`. `src/evict.rs`.
/// - `dupes` — a czkawka fallback when no fclones was present. `src/dupes.rs`; the czkawka
///   driver itself survives as `win-dupes`.
///
/// There is no pre-dispatch platform check for any engine row, and there must not be: the
/// engine owns the platform answer for every command it serves (`burrow-engine 5e42bf0` gates
/// `sentinel`'s inferred `~/.Trash`, refuses `evict` off macOS, and refuses `dupes … --apply` on
/// Windows above `resolve_fclones`), and a check here would answer for a platform whose real
/// answer lives one process away and can change without this file hearing about it. The one
/// exception is `sentinel --watch` with no directory, which never reaches the engine — see
/// `run_sentinel`.
pub const COMMANDS: &[Spec] = &[
    Spec {
        name: "status",
        aliases: &[],
        owner: Engine,
        writes: Never,
        engine_streams: &["--watch"],
        platform: Any,
        section: ReadOnly,
        usage: "status [--raw] | status --watch [--interval <secs>]",
        summary: "System snapshot; --watch streams one snapshot per tick as raw NDJSON",
    },
    Spec {
        name: "analyze",
        aliases: &[],
        owner: Engine,
        writes: Never,
        engine_streams: &["--progress"],
        platform: Any,
        section: ReadOnly,
        usage: "analyze <path> [--raw] [--progress]",
        summary: "Disk-usage tree; --progress streams running totals, then the result",
    },
    Spec {
        name: "history",
        aliases: &[],
        owner: Engine,
        writes: Never,
        engine_streams: &[],
        platform: Any,
        section: ReadOnly,
        usage: "history [--limit N]",
        summary: "Cleanup history",
    },
    Spec {
        name: "rules",
        aliases: &[],
        owner: Engine,
        writes: Never,
        engine_streams: &[],
        platform: Any,
        section: ReadOnly,
        usage: "rules [list|validate|dryrun] [dir] [--app id]",
        summary: "Declarative cleaning rules",
    },
    Spec {
        name: "orphans",
        aliases: &[],
        owner: Engine,
        writes: Never,
        engine_streams: &[],
        platform: Any,
        section: ReadOnly,
        usage: "orphans [dir] [--installed ids]",
        summary: "Leftover files belonging to no installed app",
    },
    Spec {
        name: "diff",
        aliases: &[],
        owner: Native,
        writes: Never,
        engine_streams: &[],
        platform: Any,
        section: ReadOnly,
        usage: "diff <dir>",
        summary: "Disk-growth since the previous scan of <dir>",
    },
    Spec {
        name: "snapshot",
        aliases: &[],
        owner: Native,
        writes: Never,
        engine_streams: &[],
        platform: Any,
        section: ReadOnly,
        usage: "snapshot",
        summary: "Record a health sample to history (reads the engine's status)",
    },
    Spec {
        name: "digest",
        aliases: &["report"],
        owner: Native,
        writes: Never,
        engine_streams: &[],
        platform: Any,
        section: ReadOnly,
        usage: "digest [--days N]",
        summary: "Summarize recent health history (alias: report [--weekly])",
    },
    Spec {
        name: "watch",
        aliases: &[],
        owner: Native,
        writes: Never,
        engine_streams: &[],
        platform: Any,
        section: ReadOnly,
        usage: "watch",
        summary: "Leak/runaway process alerts (reads the engine's status)",
    },
    Spec {
        name: "trigger",
        aliases: &[],
        owner: Native,
        writes: Never,
        engine_streams: &[],
        platform: Any,
        section: ReadOnly,
        usage: "trigger [--threshold N] [--rules <dir>]",
        summary: "Low-disk check; --rules plans auto-opted safe rules",
    },
    Spec {
        name: "net",
        aliases: &[],
        owner: Engine,
        writes: Never,
        engine_streams: &[],
        platform: MacOs,
        section: ReadOnly,
        usage: "net [--limit N]",
        summary: "Per-app network usage",
    },
    Spec {
        name: "sentinel",
        aliases: &[],
        owner: Engine,
        writes: Never,
        // `--watch` is the conductor's poll loop, dispatched in `run_sentinel` before the engine
        // is consulted; the engine has no watch mode and refuses the flag.
        engine_streams: &[],
        platform: Any,
        section: ReadOnly,
        usage: "sentinel [trashdir] [--watch [--interval-ms N] [--max-ticks N]]",
        summary: ".app bundles in the Trash; --watch streams NDJSON events (needs an explicit trashdir on Windows)",
    },
    Spec {
        name: "slim-check",
        aliases: &[],
        owner: Engine,
        writes: Never,
        engine_streams: &[],
        platform: MacOs,
        section: ReadOnly,
        usage: "slim-check <binary>",
        summary: "Mach-O fat slices + reclaimable bytes",
    },
    Spec {
        name: "photos",
        aliases: &[],
        owner: Engine,
        writes: Never,
        engine_streams: &[],
        platform: Any,
        section: ReadOnly,
        usage: "photos <dir> [--threshold N]",
        summary: "Visually-similar PNG/JPEG images",
    },
    Spec {
        name: "win-dupes",
        aliases: &[],
        owner: Czkawka,
        writes: Never,
        engine_streams: &[],
        platform: Windows,
        section: ReadOnly,
        usage: "win-dupes <dir> [--images]",
        summary: "Read-only duplicates / similar photos via czkawka (--apply is refused)",
    },
    Spec {
        name: "gui",
        aliases: &["app"],
        owner: Native,
        writes: Never,
        engine_streams: &[],
        platform: MacOs,
        section: ReadOnly,
        usage: "gui [--install]",
        summary: "Launch the Burrow GUI app (or install the cask)",
    },
    Spec {
        name: "clean",
        aliases: &[],
        owner: Engine,
        writes: ApplyOrDryRun,
        engine_streams: &["--stream"],
        platform: Any,
        section: Action,
        usage: "clean [--apply] [--stream] [--plan <file>]",
        summary: "Cache/junk cleanup; --stream emits live NDJSON; --apply --plan executes exactly a held preview",
    },
    Spec {
        name: "optimize",
        aliases: &[],
        owner: Engine,
        writes: ApplyOrDryRun,
        engine_streams: &["--stream"],
        platform: Any,
        section: Action,
        usage: "optimize [--apply] [--stream]",
        summary: "System cache refresh; --stream emits live NDJSON",
    },
    Spec {
        name: "purge",
        aliases: &[],
        owner: Engine,
        writes: ApplyOrDryRun,
        engine_streams: &["--stream"],
        platform: Any,
        section: Action,
        usage: "purge [--apply] [--stream]",
        summary: "Project build-artifact cleanup; --stream emits live NDJSON",
    },
    Spec {
        name: "uninstall",
        aliases: &[],
        owner: Engine,
        writes: ApplyOrDryRun,
        engine_streams: &[],
        platform: Any,
        section: Action,
        usage: "uninstall <app> [--apply] | uninstall --list",
        summary: "Remove an app + leftovers (--list enumerates installed apps)",
    },
    Spec {
        name: "installer",
        aliases: &[],
        owner: Engine,
        writes: ApplyOrDryRun,
        engine_streams: &[],
        platform: Any,
        section: Action,
        usage: "installer [--apply]",
        summary: "Remove leftover installer files",
    },
    Spec {
        name: "dupes",
        aliases: &[],
        owner: Engine,
        writes: ApplyOnly,
        engine_streams: &[],
        platform: Any,
        section: Action,
        usage: "dupes [group|dedupe|remove|link] <paths...> [--keep <dir>] [--apply]",
        summary: "Find duplicates (read-only by default); dedupe/remove/link --apply mutate (macOS/fclones-only)",
    },
    Spec {
        name: "evict",
        aliases: &[],
        owner: Engine,
        writes: ApplyOnly,
        engine_streams: &[],
        platform: MacOs,
        section: Action,
        usage: "evict <paths...> [--apply]",
        summary: "Cloud-file dehydration",
    },
    Spec {
        name: "slim",
        aliases: &[],
        owner: Native,
        writes: Writes::Native,
        engine_streams: &[],
        platform: MacOs,
        section: Action,
        usage: "slim <binary> [--apply --output P]",
        summary: "Thin a fat binary to host arch + re-sign",
    },
    Spec {
        name: "trash",
        aliases: &[],
        owner: Native,
        writes: Writes::Native,
        engine_streams: &[],
        platform: Any,
        section: Action,
        usage: "trash <paths...> | trash --apply-plan <file>  [--apply] [--stream]",
        summary: "Safe-delete to Recycle Bin (Win) / Trash (mac); --apply-plan trashes exactly a held preview, --stream emits one NDJSON event per item",
    },
    Spec {
        name: "win-uninstall",
        aliases: &[],
        owner: Bcu,
        writes: Writes::Native,
        engine_streams: &[],
        platform: Windows,
        section: Action,
        usage: "win-uninstall <app> [--confidence G] [--apply]",
        summary: "BCU preview; --apply uninstalls",
    },
];

/// The row for `name`, by canonical name or alias.
pub fn spec(name: &str) -> Option<&'static Spec> {
    COMMANDS
        .iter()
        .find(|s| s.name == name || s.aliases.contains(&name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_aliases_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for s in COMMANDS {
            assert!(seen.insert(s.name), "duplicate command {}", s.name);
            for a in s.aliases {
                assert!(seen.insert(a), "alias {a} collides");
            }
        }
    }

    #[test]
    fn aliases_resolve_to_their_canonical_row() {
        assert_eq!(spec("report").unwrap().name, "digest");
        assert_eq!(spec("app").unwrap().name, "gui");
        assert!(spec("bogus").is_none());
    }

    /// The engine's write conventions belong to engine rows and the conductor's to native rows;
    /// a row with the wrong one would be planned by `engine::plan` for a command it does not
    /// serve, or handled natively with a translation the engine never sees.
    #[test]
    fn write_conventions_match_their_owner() {
        for s in COMMANDS {
            match (s.owner, s.writes) {
                (Owner::Engine, Writes::Native) => {
                    panic!(
                        "{}: an engine row cannot have a native write convention",
                        s.name
                    )
                }
                (Owner::Engine, _) => {}
                (_, Writes::ApplyOrDryRun | Writes::ApplyOnly) => {
                    panic!("{}: only the engine translates --apply this way", s.name)
                }
                _ => {}
            }
        }
    }

    /// Streams are an engine transport; a native row listing one would be forwarded to an engine
    /// that does not serve it.
    #[test]
    fn only_engine_rows_declare_engine_streams() {
        for s in COMMANDS {
            if s.owner != Owner::Engine {
                assert!(
                    s.engine_streams.is_empty(),
                    "{}: not the engine's, cannot stream through it",
                    s.name
                );
            }
        }
    }

    /// The engine's raw streams are exactly the five it implements (BUR-132 included), each on
    /// its own row — and each row's usage line advertises the flag, so `help` cannot list a
    /// stream the conductor would capture, or hide one it forwards.
    #[test]
    fn engine_streams_are_the_five_the_engine_implements() {
        let mut streams: Vec<(&str, &str)> = COMMANDS
            .iter()
            .flat_map(|s| s.engine_streams.iter().map(move |f| (s.name, *f)))
            .collect();
        streams.sort_unstable();
        assert_eq!(
            streams,
            [
                ("analyze", "--progress"),
                ("clean", "--stream"),
                ("optimize", "--stream"),
                ("purge", "--stream"),
                ("status", "--watch"),
            ]
        );
        for (name, flag) in streams {
            let usage = spec(name).unwrap().usage;
            assert!(usage.contains(flag), "{name}: usage {usage:?} hides {flag}");
        }
    }

    /// The help tag is derived from the row, so a command cannot be labelled one engine in
    /// `help` and attributed another in its envelope.
    #[test]
    fn help_tag_names_the_owner_and_platform() {
        assert_eq!(spec("snapshot").unwrap().help_tag(), "[native]");
        assert_eq!(
            spec("net").unwrap().help_tag(),
            "[burrow-engine, macOS-only]"
        );
        assert_eq!(spec("win-uninstall").unwrap().help_tag(), "[bcu, Windows]");
    }
}
