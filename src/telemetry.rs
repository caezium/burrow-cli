//! Anonymous, opt-out usage telemetry (PostHog).
//!
//! Privacy model (Next.js/Homebrew-style, tuned for a dev + agent CLI):
//! - **Opt-out**, but with a one-time first-run notice and an obvious off switch.
//! - **Anonymous**: a random install id (not hardware-derived); events carry the COMMAND
//!   NAME ONLY — never the args, which contain paths/app-names (PII).
//! - **Off** automatically when: no key compiled in (dev/source builds), `DO_NOT_TRACK`/`CI`/
//!   `BURROW_TELEMETRY=0` set, or the run is non-interactive (piped/agent/MCP/CI) — the last
//!   both respects automation and keeps the data human-usage-only.
//! - **Fire-and-forget**: a spawned `curl` (no HTTP crate, no exit delay, fail-silent).
//!
//! Because the source is public (FSL), this stays auditable: command-names-only, no hidden
//! fields. Keys are injected at release build; absent in dev → telemetry no-ops.

use crate::platform;
use serde::{Deserialize, Serialize};
use std::io::IsTerminal;
use std::path::PathBuf;

/// PostHog capture key + host, injected at release build time (empty in dev → telemetry off).
fn posthog_key() -> &'static str {
    option_env!("BURROW_POSTHOG_KEY").unwrap_or("")
}
fn posthog_host() -> &'static str {
    option_env!("BURROW_POSTHOG_HOST").unwrap_or("https://us.i.posthog.com")
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// None = default (opt-out → on); Some(false/true) = explicit user choice.
    pub enabled: Option<bool>,
    #[serde(default)]
    pub install_id: String,
    #[serde(default)]
    pub notice_shown: bool,
}

fn config_path() -> PathBuf {
    platform::config_dir().join("telemetry.json")
}

fn random_install_id() -> String {
    use std::io::Read;
    let mut buf = [0u8; 16];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_ok()
    {
        return buf.iter().map(|b| format!("{b:02x}")).collect();
    }
    // Portable fallback (Windows has no /dev/urandom): RandomState's SipHash keys are
    // seeded from OS entropy per process, so hashing with two fresh states yields 128
    // genuinely random bits with zero extra dependencies.
    use std::hash::{BuildHasher, Hasher};
    let mut id = String::with_capacity(32);
    for _ in 0..2 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(0);
        id.push_str(&format!("{:016x}", h.finish()));
    }
    id
}

/// Load config, minting + persisting an install id on first use.
pub fn load() -> Config {
    let path = config_path();
    let mut cfg: Config = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if cfg.install_id.is_empty() {
        cfg.install_id = random_install_id();
        let _ = save(&cfg);
    }
    cfg
}

pub fn save(cfg: &Config) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(
        &path,
        serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// Whether an env var forces telemetry off (DO_NOT_TRACK / CI / BURROW_TELEMETRY).
pub fn is_env_disabled(get: &dyn Fn(&str) -> Option<String>) -> bool {
    if get("DO_NOT_TRACK")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
    {
        return true;
    }
    if get("CI")
        .map(|v| !v.is_empty() && v != "0" && v != "false")
        .unwrap_or(false)
    {
        return true;
    }
    matches!(
        get("BURROW_TELEMETRY").as_deref(),
        Some("0") | Some("false") | Some("off") | Some("no")
    )
}

/// The pure gating decision. Opt-out: default on, but gated by key/env/interactivity/choice.
pub fn decide_enabled(
    key_present: bool,
    env_disabled: bool,
    interactive: bool,
    user_choice: Option<bool>,
) -> bool {
    if !key_present || env_disabled || !interactive {
        return false;
    }
    user_choice.unwrap_or(true)
}

/// Coarse duration bucket (avoids precise-timing fingerprinting).
pub fn duration_bucket(ms: u64) -> &'static str {
    match ms {
        0..=99 => "<100ms",
        100..=999 => "<1s",
        1_000..=4_999 => "<5s",
        5_000..=29_999 => "<30s",
        _ => ">=30s",
    }
}

/// Build a PostHog capture payload for `event`, merging `extra` event-specific properties onto
/// the common ones. Command events carry the command NAME ONLY — never args. `source: "cli"`
/// lets a CLI dashboard filter these out of a shared project alongside the app.
pub fn event_json(key: &str, install_id: &str, event: &str, extra: serde_json::Value) -> String {
    let mut props = serde_json::json!({
        "cli_version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "source": "cli",
        "$lib": "burrow-cli",
    });
    if let (Some(obj), Some(ex)) = (props.as_object_mut(), extra.as_object()) {
        for (k, v) in ex {
            obj.insert(k.clone(), v.clone());
        }
    }
    serde_json::json!({
        "api_key": key,
        "event": event,
        "distinct_id": install_id,
        "properties": props,
    })
    .to_string()
}

/// Fire-and-forget a payload via a spawned curl (never waits, never surfaces errors).
fn post(body: &str) {
    let url = format!("{}/capture/", posthog_host());
    let _ = std::process::Command::new("curl")
        .args([
            "-s",
            "-m",
            "2",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-d",
            body,
            &url,
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn live_enabled(cfg: &Config) -> bool {
    decide_enabled(
        !posthog_key().is_empty(),
        is_env_disabled(&|k| std::env::var(k).ok()),
        std::io::stdout().is_terminal(),
        cfg.enabled,
    )
}

/// Show the one-time first-run notice (only when telemetry is actually live).
pub fn maybe_first_run_notice() {
    let mut cfg = load();
    if cfg.notice_shown || !live_enabled(&cfg) {
        return;
    }
    eprintln!(
        "burrow collects anonymous usage (command names only — never paths or file names) to \
         guide development.\nIt's on by default; turn it off anytime: `burrow telemetry off`  \
         (or set DO_NOT_TRACK=1)."
    );
    cfg.notice_shown = true;
    let _ = save(&cfg);
    // One-time install event — drives the installs metric (separate from per-command usage).
    post(&event_json(
        posthog_key(),
        &cfg.install_id,
        "cli_installed",
        serde_json::json!({}),
    ));
}

/// Record one command invocation (fire-and-forget). No-op unless live.
pub fn record(command: &str, success: bool, ms: u64) {
    let cfg = load();
    if !live_enabled(&cfg) {
        return;
    }
    post(&event_json(
        posthog_key(),
        &cfg.install_id,
        "command",
        serde_json::json!({ "command": command, "success": success, "duration": duration_bucket(ms) }),
    ));
}

/// `burrow telemetry on|off|status`.
pub fn command(args: &[String]) -> String {
    let sub = args.first().map(String::as_str).unwrap_or("status");
    let mut cfg = load();
    match sub {
        "on" => {
            cfg.enabled = Some(true);
            let _ = save(&cfg);
            "telemetry: enabled".into()
        }
        "off" => {
            cfg.enabled = Some(false);
            let _ = save(&cfg);
            "telemetry: disabled".into()
        }
        _ => {
            let env_disabled = is_env_disabled(&|k| std::env::var(k).ok());
            let key = !posthog_key().is_empty();
            let interactive = std::io::stdout().is_terminal();
            let live = live_enabled(&cfg);
            format!(
                "telemetry: {}\n  user choice: {}\n  build has key: {}\n  env-disabled (DO_NOT_TRACK/CI/BURROW_TELEMETRY): {}\n  interactive: {}\n  install id: {}",
                if live { "ACTIVE" } else { "inactive" },
                match cfg.enabled { Some(true) => "on", Some(false) => "off", None => "default (on)" },
                key, env_disabled, interactive, cfg.install_id
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_ids_are_32_hex_and_distinct() {
        // Portable randomness (Windows has no /dev/urandom): every platform must mint a
        // well-formed, non-repeating id — a timestamp-collision id would merge installs.
        let a = random_install_id();
        let b = random_install_id();
        assert_eq!(a.len(), 32, "{a}");
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()), "{a}");
        assert_ne!(a, b, "two mints must differ");
    }

    #[test]
    fn opt_out_default_on_but_gated() {
        // key + not-env-disabled + interactive + no explicit choice => ON (opt-out default).
        assert!(decide_enabled(true, false, true, None));
        // explicit off wins.
        assert!(!decide_enabled(true, false, true, Some(false)));
        // each gate disables it.
        assert!(!decide_enabled(false, false, true, None), "no key");
        assert!(!decide_enabled(true, true, true, None), "env disabled");
        assert!(
            !decide_enabled(true, false, false, None),
            "non-interactive/agent"
        );
    }

    #[test]
    fn env_disable_rules() {
        let on = |pairs: &[(&str, &str)]| {
            let m: std::collections::HashMap<String, String> = pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            is_env_disabled(&|k| m.get(k).cloned())
        };
        assert!(on(&[("DO_NOT_TRACK", "1")]));
        assert!(!on(&[("DO_NOT_TRACK", "0")]));
        assert!(on(&[("CI", "true")]));
        assert!(on(&[("BURROW_TELEMETRY", "off")]));
        assert!(!on(&[("BURROW_TELEMETRY", "1")]));
        assert!(!on(&[]));
    }

    #[test]
    fn command_event_is_name_only_no_args() {
        let e = event_json(
            "phc_test",
            "abc123",
            "command",
            serde_json::json!({ "command": "uninstall", "success": true, "duration": "<5s" }),
        );
        assert!(e.contains("\"command\":\"uninstall\""));
        assert!(e.contains("\"duration\":\"<5s\""));
        assert!(
            e.contains("\"source\":\"cli\""),
            "tagged for the CLI dashboard"
        );
        assert!(e.contains("burrow-cli"));
        assert!(!e.contains('/'), "must not contain any path: {e}");
    }

    #[test]
    fn install_event_shape() {
        let e = event_json("k", "id", "cli_installed", serde_json::json!({}));
        assert!(e.contains("\"event\":\"cli_installed\""));
        assert!(e.contains("\"source\":\"cli\""));
        assert!(e.contains("\"cli_version\":"));
    }

    #[test]
    fn duration_buckets() {
        assert_eq!(duration_bucket(50), "<100ms");
        assert_eq!(duration_bucket(1500), "<5s");
        assert_eq!(duration_bucket(60_000), ">=30s");
    }

    #[test]
    fn config_roundtrip() {
        let dir = std::env::temp_dir().join(format!("burrow_tele_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("BURROW_CONFIG_DIR", &dir);
        let mut c = load();
        assert!(!c.install_id.is_empty(), "install id minted");
        c.enabled = Some(false);
        save(&c).unwrap();
        assert_eq!(load().enabled, Some(false));
        std::env::remove_var("BURROW_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
