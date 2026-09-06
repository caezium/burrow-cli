# Burrow CLI — Windows compatibility status

This public `windows-legacy` snapshot retains the earlier CLI used by Burrow's Windows app.
It uses the Mole/digger adapter contract, not the FSL Rust engine protocol used on `main`.

The app currently reaches this CLI only for fixed clean/optimize requests, with confirmation
and destructive opt-in enforced by the app. Its vendored MIT PowerShell engine supplies those
commands. Other Windows UI features use native app services.

## Reviewed compatibility fixes

Batch targets are launched through Rust's standard batch adapter. Literal percent sequences
in the program path, double quotes in arguments, and CR/LF are refused before launch to prevent
command substitution or loss of the trailing preview guard. The explicit legacy `cmd /C`
wrapper is removed. The existing command mapping and dry-run/apply inversion are preserved.

## Validation

The baseline passes 100 unit tests and 28 integration tests on macOS. Windows/macOS/Linux CI
passes formatting, Clippy and tests; Windows runs four additional process tests covering real
cmd/bat forwarding, portable installation paths, app preview/apply mapping, and refusal before
an unrepresentable input can launch a script. License checks pass.

The retained Windows-native inventory, network, cloud-file and optional BCU/czkawka adapters
remain separate from the app's fixed clean/optimize call surface. The broader native/rules fixes
on the newer CLI `main` are not part of this narrow compatibility backport. Real Windows system
maintenance and installed third-party tools require separate platform acceptance testing.

[PROVENANCE.md](PROVENANCE.md) retains upstream attribution. Private machine measurements,
build-session logs, telemetry keys, and prior Git history are excluded from this publication.
