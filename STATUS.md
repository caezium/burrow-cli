# Burrow CLI — reviewed source status

This `main` snapshot uses the separate FSL-1.1-ALv2 Rust `burrow-engine` process. The public
`windows-legacy` branch carries the Windows app's older Mole adapter contract independently.
See [README.md](README.md#platform-support) for the platform matrix and
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for dispatch and output contracts.

## Included review fixes

- Native write commands reject malformed flags before acting. `slim` requires a new output
  and successful signing; trash plans are fully validated and partial failures return nonzero.
- Automation refuses rule conditions/actions/searches it cannot preserve, instead of silently
  expanding them into whole-path deletion.
- Missing sentinel/diff scan roots fail. Engine classifications, details and stream refusals
  survive forwarding. Invalid JSON and malformed duplicate groups are errors.
- Windows batch scripts use Rust's batch handling. Percent-bearing program paths and quoted
  or multiline arguments are refused before launch because they cannot preserve the preview
  guard through the packaged PowerShell adapter.

## Verification at the reviewed baseline

The macOS suite passes 131 unit tests and 58 integration tests. Linux/macOS/Windows CI passes
formatting, Clippy, tests, and license checks. Windows additionally runs real cmd/bat fixture
processes for argument forwarding and preview/apply behavior. The Windows release binary is
built in CI; no maintenance command or release publication is part of those tests.

Engine-backed integration was checked using isolated temporary fixtures and previews. Actual
administrator prompts, real system cleanup, signing/notarization and installed third-party
Windows tools require separate platform acceptance checks.

No private machine measurements, build-session logs, credentials or prior Git history are
included in this status record. [PROVENANCE.md](PROVENANCE.md) retains upstream attribution.
