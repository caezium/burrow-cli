# Telemetry

Burrow CLI collects **anonymous, opt-out** usage analytics to guide development — the
Next.js/Homebrew model: on by default, a one-time first-run notice, trivially disabled.

The source is public (FSL), so this is fully auditable — the exact payload is in
[`src/telemetry.rs`](src/telemetry.rs).

## What's collected (per command)

Only:
- the **command name** (e.g. `clean`, `dupes`, `status`) — **never the arguments**
- success / failure
- a coarse duration bucket (`<100ms`, `<1s`, `<5s`, …)
- CLI version, OS, CPU architecture
- a random, locally-generated install id (**not** derived from hardware)

## What is NEVER collected

- **File paths, file names, app names, rule contents** — anything in command arguments.
  (`uninstall Foo.app` records `command: "uninstall"`, never `Foo`.)
- Any personal or machine-identifying data. No IP-based identity — a random id only.

## Automatically OFF when

- a dev/source build (no analytics key compiled in);
- `DO_NOT_TRACK=1`, `CI=true`, or `BURROW_TELEMETRY=0` is set;
- the run is **non-interactive** (piped, agent/MCP-driven, CI) — only interactive human use
  is counted, which also keeps automation noise out of the data.

## Turn it off

```sh
burrow telemetry off       # persistent (stored in ~/.burrow/telemetry.json)
# or, per-run / per-environment:
DO_NOT_TRACK=1 burrow ...
```

`burrow telemetry status` shows the current state and *why*; `burrow telemetry on` re-enables.

## Events

- **`cli_installed`** — fired once, on the first interactive run → the **installs** metric.
- **`command`** — one per command invocation → the **usage** metric (name only, never args).

## Where it goes

Release maintainers can configure a PostHog capture project at build time with
`BURROW_POSTHOG_KEY` and `BURROW_POSTHOG_HOST`. No live project key is included in this source
snapshot; `.release.env.example` contains placeholders. A build without a key has telemetry
disabled. The default capture host is `https://us.i.posthog.com`.

Configured builds send events with a fire-and-forget HTTPS POST (a spawned `curl` with a
2-second timeout; failures are ignored). Events carry `source: "cli"` and `$lib: "burrow-cli"`
so maintainers can distinguish CLI events in their own project.
