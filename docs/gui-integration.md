# GUI and MCP integration

The [Burrow app](https://github.com/caezium/Burrow) owns GUI authorization, MCP tool exposure,
and packaged runtime selection. Its macOS implementation runs the Rust engine directly.
Its Windows implementation launches the `windows-legacy` CLI with an argument vector, sets
`BURROW_ENGINE_DIR` to its bundled Mole directory, and parses the CLI envelope.

For buffered commands, consumers branch on `ok` and read `data` or `error`. Native stream
modes emit NDJSON and bypass the buffered envelope. The caller must request a stream only
when it implements that command's stream protocol and must retain the subprocess exit status.

A destructive CLI command previews by default. Only an explicitly authorized apply request
adds `--apply`. An operation that fails after starting must not be retried through another
backend automatically: a second invocation could repeat a partially completed mutation.

## Runtime selection

This branch supplies the Windows app's older adapter contract. The app bundles
`Assets/burrow.exe` from `windows-legacy` and sets `BURROW_ENGINE_DIR` to `Assets/Mole`.
`burrow-engine.cmd` invokes the vendored MIT PowerShell clean/optimize commands. The app's
native services handle its other Windows UI features.

Keep the CLI pin and Mole runtime together. CLI `main` uses the newer Rust engine protocol,
so repinning to `main` alone does not migrate a Windows package. The public branch names
describe the two contracts rather than two interchangeable builds of one runtime.

## Verification

The CLI suite tests clean/optimize preview/apply shaping with fake engines. Windows CI adds
real cmd/bat process tests for metacharacters, portable paths and preview-guard preservation.
The app repository tests MCP confirmation mapping, restored runtime packaging and mocked
PowerShell operations under Windows PowerShell 5.1; it owns platform acceptance testing.
