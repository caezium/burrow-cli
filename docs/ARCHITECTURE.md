# Windows compatibility CLI — architecture

The `windows-legacy` branch preserves the process contract required by the Windows app. It is
a separately reviewed source snapshot; replacing it with `main` also requires an engine
migration. Both CLI branches retain FSL-1.1-ALv2 in `LICENSE.md`.

## Process boundaries

The Rust conductor owns command selection, preview/apply translation and JSON envelopes.
Legacy engine commands run as separate processes: a Mole/digger entrypoint for Bash/PowerShell
commands, with optional `status-go` and `analyze-go` binaries for their older JSON contracts.
The Windows app supplies `Assets/Mole/burrow-engine.cmd` and its MIT PowerShell runtime.

The historical Bash/Go fork is `caezium/burrow-digger`, formerly named `burrow-engine`, based on
Mole at `9daf936ea5fd1a0648579434c76000cd9cfa1253` under its preserved MIT notice. The present
`caezium/burrow-engine` repository is a distinct FSL Rust engine used by CLI `main`.

Optional fclones, czkawka and BCU tools are separate processes. Rust modules retain the older
inventory, network, cloud-file, photo, rule, diff, trash and metrics implementations; their
source and license provenance is recorded in `PROVENANCE.md` and `THIRD-PARTY-LICENSES.md`.

## Windows app boundary

The application calls this conductor for clean/optimize with a fixed command name and flags.
The default inserts the legacy engine's `--dry-run`; explicit `--apply` omits that flag.
The app owns confirmation and destructive opt-in. It passes `BURROW_ENGINE_DIR` explicitly so
its bundled adapter is selected; a failed started command must not be automatically retried.

`platform::command` hands batch programs to Rust's standard batch handler, preserving its
quoting and disabled AutoRun/delayed expansion. It refuses percent-bearing batch program paths
and quoted/multiline arguments before launch because the PowerShell forwarding layer cannot
represent those inputs without changing subsequent arguments. Native executable arguments are
unchanged. `tests/windows_batch.rs` validates these boundaries using harmless fixture programs.

## State and output

Windows uses AppData/LocalAppData for CLI state; macOS uses `~/.burrow`. Overrides used by tests
are centralized in `platform.rs`. Buffered commands return `{ok,command,data|error}` envelopes;
process failures remain failures. The CLI itself does not host an MCP server; the Burrow app
owns that interface and its authorization policy.
