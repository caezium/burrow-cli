# Burrow CLI

Agent-native macOS system-cleaning CLI — the scriptable, transparent
command-line sibling of [Burrow](https://github.com/caezium/Burrow), sharing its Rust engine.

> **Status:** v0. Written in **Rust**. Requires the separate FSL Rust `burrow-engine` runtime.
> 26 commands implemented + tested (engine passthrough, dupes, rules, orphans, diff,
> snapshot/digest, watch/trigger, slim, evict, photos, net, sentinel, trash, the two
> Windows adapters, GUI launcher — plus `version`/`help`/`telemetry`), verified against
> the real engine build. Engine-backed commands follow the current Rust engine's platform
> support — see [Platform support](#platform-support),
> [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) and [`STATUS.md`](STATUS.md).

## What it is

Burrow CLI orchestrates a small set of cleaning **engines** behind one explain-before-delete,
JSON-emitting command surface that humans *and* AI agents can drive. It does **not** reinvent
cleaning primitives — it conducts proven engines and adds the differentiated layer: a
declarative, provenance-carrying rule database, snapshot diffing, and JSON output for agent integrations. The app owns the MCP server.

## Platform support

Burrow CLI targets macOS and also builds and runs on Windows; CI produces a
`burrow-windows-x86_64` artifact every push. The current Rust engine implements Windows `net`
through IP Helper with a `netstat` fallback, reporting per-process connection counts. Registry
orphan inventory and OneDrive eviction remain unavailable in this runtime, and `dupes` uses
fclones without the older czkawka fallback. The supported commands and refusals are listed below.

The public `windows-legacy` branch retains the earlier adapter contract required by the
Windows app's bundled Mole PowerShell runtime. It is a separate source snapshot.

A refusal is a *failure envelope*, never an empty success, so a caller can tell "not available
here" from "nothing found". The distinction is the point — an `orphans` run reporting zero
leftovers and an `orphans` run that could not enumerate installed apps at all are very different
answers, and only one of them was ever true on Windows. Two shapes, and which one you get tells
you who refused:

```jsonc
// refused by the conductor, before the engine runs — note the top-level "feature"
{"ok": false, "command": "slim", "error": {"kind": "unsupported", "message": …}, "feature": "slim"}

// refused by the engine, relayed — no "feature" key; the engine's own message is carried through
{"ok": false, "command": "evict", "error": {"kind": "unsupported", "message": …}}
```

**Refused before the engine is even spawned.** This group used to hold `evict` and `sentinel` too,
as an interim while the engine answered those two *wrongly* off macOS rather than not at all;
`burrow-engine 5e42bf0` fixed both at the source, so they moved down to the next table and
`engine::windows_refusal` was retired. What is left is commands the engine never had:

| Command | Why | What you get |
|---|---|---|
| `sentinel --watch` with no directory | `--watch` has no engine equivalent at all — the poll loop is the conductor's — so the engine's gate on the same inferred `~/.Trash` cannot cover it. Same scope as the engine's: only the path you did *not* name | `ok:false`, `kind:"unsupported"` |
| `slim`, `gui` | Mach-O thinning + `codesign`, and launching the macOS app | `ok:false`, `kind:"unsupported"` |

**Refused by the engine**, relayed through the conductor's error envelope:

| Command | What you get |
|---|---|
| `orphans` | `ok:false`, `kind:"unsupported"` — "installed-app inventory unavailable". The `reg query` walk over the three Uninstall roots went with `src/orphan.rs`. **`orphans <dir> --installed <ids>` still runs**: supplying the inventory yourself skips the enumeration the engine can't do, and the directory scan itself has no platform gate. Its matching heuristics are bundle-id-shaped though, so expect thin results against Windows filenames. |
| `uninstall --list` | `ok:false`, `kind:"unsupported"` — "app inventory is macOS only", same missing source. |
| `evict` (preview **and** `--apply`) | `ok:false`, `kind:"unsupported"` — "cloud-file dehydration needs macOS brctl". Both halves refuse with one envelope: the preview used to return `ok:true` while marking every item `supported:false`, which a caller branching on `ok` reads as a successful preview. The provider-aware OneDrive path (`attrib +U -P`) went with `src/evict.rs`. |
| `dupes dedupe\|remove\|link --apply` | `ok:false`, `kind:"unsupported"` — "Windows duplicate discovery is read-only; dedupe/remove/link apply actions are macOS/fclones-only". **"Burrow does not delete duplicate files on Windows" holds again.** The guard sits *above* fclones resolution, so `$BURROW_FCLONES` or an `fclones.exe` on `PATH` does not buy the mutation back, and it is duplicated inside the engine's `execute` because burrow-cli is not its only caller. Read-only `dupes` and the `--dry-run` previews are untouched. |
| `sentinel` with no directory | `ok:false`, `kind:"unsupported"` — the inferred `<home>/.Trash` is a macOS path, so a scan of it here would report an empty trash it never opened. Only the *inference* is refused; `sentinel <dir>` is served on every platform. (If neither `HOME` nor `USERPROFILE` is set you get `kind:"not_found"` instead — the more specific answer wins.) |
| `slim-check` | `ok:false`, `kind:"error"` — the engine reads the header and reports "not a fat Mach-O (thin binary or non-Mach-O)" for a PE. The engine's own wording table classifies it `error`, and the conductor relays that; it is a plain honest failure, not a fall-through. |

**Works on Windows:**

- **`net`** — native IP Helper TCP/UDP IPv4/IPv6 owner tables provide per-process connection
  counts. If the native collector fails, the engine falls back to `netstat -ano` and `tasklist`.
  `total` contains the connection count; `bytes_in` and `bytes_out` are zero. `metric_source`
  and `metric_note` identify the collector and explain that these are not bandwidth counters.
  If both collectors fail, the CLI returns an error with both failure reasons.
- **`dupes`** (read-only `group`) — through `fclones`, which is genuinely cross-platform. The
  engine resolves `$BURROW_FCLONES` then `fclones` on `PATH` (`.exe` handled), so this works with
  a normal `cargo install fclones`. Without one: `ok:false`, `kind:"not_found"`, message naming
  the install. The czkawka fallback `dupes` used to have on Windows is gone — czkawka is now
  reachable only through `win-dupes`.
- **`dupes dedupe|remove|link` without `--apply`** — fclones' own `--dry-run` preview, which is
  read-only and runs anywhere fclones does. Only the `--apply` is refused; see the table above.
- **`sentinel <dir>`** — a `read_dir` and a `.app` suffix test, which needs no platform
  vocabulary. Only the inferred `~/.Trash` is refused.
- **`photos`**, **`rules`** — pure Rust in the engine with no platform-specific code; these had
  no Windows-specific implementation to lose.
- **`trash`** — Recycle Bin via the `trash` crate, conductor-native.
- **`diff`**, **`digest`**/`report` — conductor-native, no platform dependency.
- **`snapshot`**, **`watch`**, **`trigger`** — conductor-native, but each reads the engine's
  `status` first, and `status` refuses (`kind:"unsupported"`) when none of its probes could be
  measured, which is the expected Windows case. You get that refusal rather than a health score
  computed from zeroes.
- **`sentinel --watch <dir>`** — polls the directory you named. You asked for that path
  explicitly, so it is watched; only the implicit `~/.Trash` default is refused.
- **`win-dupes`**, **`win-uninstall`** — unchanged, see below.

## Windows duplicate and uninstall adapters

The two `win-` commands are conductor-native and were not touched by the engine migration.

`burrow win-dupes <dir> [--images]` drives `czkawka_cli` for duplicate and similar-image
discovery. Results are normalized into stable groups containing file paths and sizes, aggregate
counts, and redundant bytes when they can be derived. If czkawka is missing, Burrow emits the
exact planned argument vector as a non-destructive preview. Discovery is read-only —
czkawka has no fed-back delete here, and `--apply` returns `unsupported` on every platform.

`burrow win-uninstall <app>` is a preview by default and plans the read-only BCU `list`
command. Only explicit `--apply` runs `BCU-console.exe uninstall <app> /Q /U /J=<confidence>`.
Missing BCU, non-zero exits, access denial, and UAC/elevation denial produce `ok:false` envelopes
with a classified error and narrowly scoped `error.details`: the program, arguments, exit code,
stdout and stderr, plus `engine` and `applied`.

## Engines (wrapped as separate processes, not reinvented)

| Engine | Role | License |
|---|---|---|
| `burrow-engine` | clean / uninstall / optimize / analyze / status / purge / installer / history, plus the eight it took over: net / orphans / evict / dupes / slim-check / sentinel / photos / rules | FSL-1.1-ALv2 — [the Rust core](https://github.com/caezium/burrow-engine), with its own upstream notices; distinct from the historical MIT `caezium/burrow-digger` fork |
| fclones | duplicates + APFS clone-dedupe — the only `dupes` backend on every platform | MIT |
| czkawka *(Windows)* | discovery-only duplicates + similar photos, via `win-dupes` only | MIT (core only — not the GPL Krokiet UI) |
| Bulk Crap Uninstaller *(Windows)* | preview/apply uninstall + confidence scoring, via `win-uninstall` | Apache-2.0 |

See [`NOTICE`](NOTICE) and [`THIRD-PARTY-LICENSES.md`](THIRD-PARTY-LICENSES.md).

## License

Burrow CLI is source-available under the **Functional Source License 1.1 (Apache-2.0 future
license)** — [`LICENSE.md`](LICENSE.md). The license restricts making the software available
to others through a commercial product or service that meets its definition of **Competing
Use**. Each version gains an additional Apache-2.0 license on the second anniversary of the
date that version is made available. The full terms and permitted purposes are in the license.

The `burrow.rules` format and community rule data (under `rules/`) will be
**Apache-2.0** so the rule ecosystem can grow without friction.

## Sibling repos

- [`caezium/Burrow`](https://github.com/caezium/Burrow) — the desktop app: macOS runs the Rust engine directly; Windows uses this repository's `windows-legacy` branch.
- [`caezium/burrow-engine`](https://github.com/caezium/burrow-engine) — the Rust engine this
  CLI's cleaning commands drive, under FSL-1.1-ALv2. The macOS GUI drives the same binary.
- [`windows-legacy`](https://github.com/caezium/burrow-cli/tree/windows-legacy) — a separate
  compatibility snapshot used by the Windows app with its vendored MIT Mole PowerShell engine.

## Building the source snapshot

```sh
cargo build --locked
cargo test --locked --all-targets
```

Build and install `burrow-engine` separately for engine-backed commands, then use
`BURROW_ENGINE` or `BURROW_ENGINE_DIR` to select it. The CLI has no private Cargo dependency.
Source builds have telemetry disabled unless a maintainer supplies an analytics key at build
time; `.release.env.example` contains placeholders only.

This public repository starts from reviewed source. Earlier development history is retained
in a private archive; historical PR/commit references in provenance documents describe that
history and do not imply those objects exist in this repository.

## No copyleft, by design

Every dependency is permissively licensed — MIT/Apache-2.0, with some Zlib/BSD/Unlicense/
Unicode-3.0 in the tree (audited, allowlisted in [`deny.toml`](deny.toml)); nothing copyleft.
No GPL/AGPL code is incorporated. This is enforced in CI
([`.github/workflows/license-scan.yml`](.github/workflows/license-scan.yml)); ideas taken
from copyleft tools are reimplemented clean-room and logged in [`PROVENANCE.md`](PROVENANCE.md).
