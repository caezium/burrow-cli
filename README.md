# Burrow CLI

This is the `windows-legacy` compatibility snapshot used by the
[Burrow Windows app](https://github.com/caezium/Burrow). It preserves the earlier Mole/digger
process contract and the reviewed Windows batch safety fixes. The `main` branch instead uses
the newer FSL Rust `burrow-engine`; these runtime contracts are not interchangeable.

Build with `cargo build --locked`. Source builds contain no live analytics key. Configure an
optional key through build environment variables; `.release.env.example` contains placeholders.

## What it is

Burrow CLI orchestrates a small set of cleaning **engines** behind one explain-before-delete,
JSON-emitting command surface that humans *and* AI agents can drive. It does **not** reinvent
cleaning primitives — it conducts proven engines and adds the differentiated layer: a
declarative, provenance-carrying rule database, snapshot diffing, and JSON output for agent integrations. The app owns the MCP server.

## Platform support

The Windows parity branch keeps the same command surface and JSON envelopes while swapping in
Windows-native adapters where macOS uses platform tools:

| Area | macOS | Windows |
|---|---|---|
| Engine commands | `burrow-engine`/`mole`, `status-go`, `analyze-go` | `burrow-engine.cmd` for the app's clean/optimize runtime; optional `status-go.exe` / `analyze-go.exe` for separate compatible engines |
| Duplicate discovery | `fclones` | `czkawka_cli` by default when `BURROW_FCLONES` is unset |
| Safe delete | Trash via `trash` crate | Recycle Bin via `trash` crate |
| Orphan inventory | `/Applications` + `~/Applications` | Registry uninstall inventory + AppData/ProgramData/Program Files roots |
| Network attribution | `nettop` byte counters | `netstat -ano` + `tasklist` fallback; ETW/IP Helper remains planned |
| Cloud dehydration | `brctl evict` for iCloud | `attrib +U -P` for OneDrive/Cloud Files candidates |
| App slimming | Mach-O + `codesign` | structured `unsupported` in this branch |
| GUI launcher | Burrow.app/Homebrew cask | structured `unsupported`; CLI only |

Platform-specific unsupported cases emit a machine-readable envelope instead of silently
pretending success.

## Engines (wrapped as separate processes, not reinvented)

| Engine | Role | License |
|---|---|---|
| legacy Mole / `burrow-digger` | Bash/Go command contract; the Windows app supplies its own MIT PowerShell clean/optimize runtime | MIT — historical Mole source and notices retained; distinct from the FSL Rust engine |
| fclones | duplicates + APFS clone-dedupe | MIT |
| czkawka *(Windows)* | duplicates + similar photos | MIT (core only — not the GPL Krokiet UI) |
| Bulk Crap Uninstaller *(Windows)* | uninstall + confidence scoring | Apache-2.0 |

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
- `caezium/burrow-digger` — the historical MIT Mole fork, formerly named `burrow-engine`.
- [`caezium/burrow-engine`](https://github.com/caezium/burrow-engine) — the distinct FSL Rust runtime used by CLI `main`, not this compatibility branch.

## No copyleft, by design

Every dependency is MIT/Apache. No GPL/AGPL code is incorporated. This is enforced in CI
([`.github/workflows/license-scan.yml`](.github/workflows/license-scan.yml)); ideas taken
from copyleft tools are reimplemented clean-room and logged in [`PROVENANCE.md`](PROVENANCE.md).

## Windows app contract

The app sets `BURROW_ENGINE_DIR` to `Assets/Mole`, whose `burrow-engine.cmd` forwards into the
bundled PowerShell scripts. Unconfirmed clean/optimize calls receive `--dry-run`; explicit
`--apply` removes that preview flag. The application owns confirmation and destructive opt-in.

Rust's batch handling protects shell metacharacters. Batch program paths containing `%` and
arguments containing double quotes or newlines are refused before launch because they cannot
preserve the preview guard through PowerShell forwarding. Native `.exe` arguments retain their
normal argument-vector behavior. Windows CI tests this with harmless fake cmd/bat programs.

This branch has its own fresh source root. The earlier private development history is retained
in an archive and is not published here. Current review fixes for the newer `main` CLI are not
implicitly backported into this older compatibility surface.
