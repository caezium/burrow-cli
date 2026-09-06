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

The `main` CLI and macOS app use the separate FSL Rust `burrow-engine` runtime. The CLI
forwards the engine's error classification and details. `--raw` unwraps a buffered payload
at the conductor; unsupported stream flags reach the engine and its refusal is retained.

The Windows app uses the separate public `windows-legacy` branch with its MIT Mole PowerShell
runtime. Do not change that pin to `main` without migrating the runtime contract as well.

## Verification

`tests/cli.rs` checks envelope/stream dispatch with fixtures and isolated temporary files.
`tests/windows_batch.rs` exercises batch refusal boundaries on Windows. App integration
and authorization tests live in the app repository; this CLI has no standalone MCP server.
