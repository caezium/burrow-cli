//! Stable Burrow output envelope.
//!
//! Every conductor command emits a versioned envelope so agents get a consistent shape
//! regardless of which engine served the request. v0 embeds the engine's already-valid JSON
//! verbatim as `data`; non-JSON engine output is wrapped as `{"text": "..."}`. Zero-dep.

/// Wrap engine output in the Burrow envelope, auto-detecting JSON vs plain text.
pub fn wrap(command: &str, engine_out: &str) -> String {
    let t = engine_out.trim();
    if t.starts_with('{') || t.starts_with('[') {
        envelope(command, t)
    } else {
        envelope_text(command, t)
    }
}

/// Wrap already-valid engine JSON in the Burrow envelope (`data` = the JSON, verbatim).
pub fn envelope(command: &str, engine_json: &str) -> String {
    format!(
        "{{\"ok\":true,\"burrow_cli\":\"{}\",\"engine\":\"burrow-engine\",\"command\":\"{}\",\"data\":{}}}",
        env!("CARGO_PKG_VERSION"),
        command,
        engine_json.trim()
    )
}

/// Wrap arbitrary engine text (e.g. a bash dry-run report) as `data.text`.
pub fn envelope_text(command: &str, text: &str) -> String {
    format!(
        "{{\"ok\":true,\"burrow_cli\":\"{}\",\"engine\":\"burrow-engine\",\"command\":\"{}\",\"data\":{{\"text\":{}}}}}",
        env!("CARGO_PKG_VERSION"),
        command,
        json_string(text)
    )
}

/// Classify an error message into a coarse machine-readable `kind` so a GUI can
/// react (prompt for permissions vs show "unavailable here" vs a generic error).
/// Wording-based; folded in from Ltcc0's #5. A kind set at the error source would
/// be sturdier, but this covers the conductor's current error strings.
fn error_kind(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();
    if lower.contains("permission denied")
        || lower.contains("access is denied")
        || lower.contains("os error 5")
    {
        "permission_denied"
    } else if lower.contains("unsupported")
        || lower.contains("macos only")
        || lower.contains("windows only")
        || lower.contains("not available")
        || lower.contains("unavailable")
    {
        "unsupported"
    } else if lower.contains("not found")
        || lower.contains("could not locate")
        || lower.contains("does not exist")
        || lower.contains("is not a directory")
    {
        "not_found"
    } else if lower.contains("exited") {
        "process_failed"
    } else {
        "error"
    }
}

/// The structured error payload shared by the error/unsupported envelopes:
/// `{ "kind": …, "message": …, "platform": … }`.
fn error_object(kind: &str, message: &str) -> String {
    format!(
        "{{\"kind\":{},\"message\":{},\"platform\":\"{}\"}}",
        json_string(kind),
        json_string(message),
        std::env::consts::OS
    )
}

/// Wrap a failure — top-level `ok:false` plus a structured `error {kind, message,
/// platform}` — so a GUI/agent parses ONE shape, branches on `ok`, and gets a
/// classified reason. (Combines #4's ok-branching with #5's error classification.)
pub fn error_envelope(command: &str, message: &str) -> String {
    format!(
        "{{\"ok\":false,\"burrow_cli\":\"{}\",\"engine\":\"burrow-engine\",\"command\":\"{}\",\"error\":{}}}",
        env!("CARGO_PKG_VERSION"),
        command,
        error_object(error_kind(message), message)
    )
}

/// A platform-unsupported failure — a failure envelope whose error `kind` is
/// `unsupported`, plus the `feature` that's unavailable on this platform.
pub fn unsupported_envelope(command: &str, feature: &str, detail: &str) -> String {
    format!(
        "{{\"ok\":false,\"burrow_cli\":\"{}\",\"engine\":\"burrow-engine\",\"command\":\"{}\",\"error\":{},\"feature\":{}}}",
        env!("CARGO_PKG_VERSION"),
        command,
        error_object("unsupported", detail),
        json_string(feature)
    )
}

/// Encode a Rust string as a valid JSON string literal (zero-dep).
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_wraps_object() {
        let e = envelope("status", "{\"a\":1}");
        assert!(e.starts_with('{') && e.ends_with('}'));
        assert!(e.contains("\"command\":\"status\""));
        assert!(e.contains("\"engine\":\"burrow-engine\""));
        assert!(e.contains("\"data\":{\"a\":1}"));
    }

    #[test]
    fn wrap_detects_array() {
        assert!(wrap("status", " [1,2] ").contains("\"data\":[1,2]"));
    }

    #[test]
    fn wrap_text_is_escaped_json() {
        let e = wrap("clean", "Would remove:\n\t\"~/Library/Caches\"");
        assert!(e.contains("\"data\":{\"text\":"));
        // newline, tab, and quote must be escaped so the envelope stays valid JSON.
        assert!(e.contains("\\n"));
        assert!(e.contains("\\t"));
        assert!(e.contains("\\\""));
    }

    #[test]
    fn json_string_escapes_control_chars() {
        assert_eq!(json_string("a\u{0001}b"), "\"a\\u0001b\"");
    }

    #[test]
    fn envelope_carries_ok_true_for_success() {
        // A GUI/agent branches on a single top-level `ok` for EVERY response;
        // success must carry ok:true (mirroring the failure envelope's ok:false)
        // so the consumer parses one shape, not two.
        let e = envelope("status", "{\"a\":1}");
        assert!(
            e.contains("\"ok\":true"),
            "success envelope must carry ok:true, got: {e}"
        );
    }

    #[test]
    fn error_envelope_is_a_classified_failure() {
        // Failure = top-level ok:false + a structured error {kind, message, platform}.
        let e = error_envelope("clean", "engine \"mole\" not found");
        assert!(e.contains("\"ok\":false"), "got: {e}");
        assert!(e.contains("\"command\":\"clean\""), "got: {e}");
        assert!(e.contains("\"kind\":\"not_found\""), "classified: {e}");
        // the message is a valid, escaped JSON string nested under `error`.
        assert!(
            e.contains("\"message\":\"engine \\\"mole\\\" not found\""),
            "got: {e}"
        );
        assert!(e.contains("\"platform\":"), "got: {e}");
    }

    #[test]
    fn error_envelope_classifies_permission_denied() {
        let e = error_envelope("status", "Access is denied. (os error 5)");
        assert!(e.contains("\"kind\":\"permission_denied\""), "got: {e}");
    }

    #[test]
    fn unsupported_envelope_is_a_failure_not_success() {
        // Regression: a platform-unsupported response must be top-level ok:false
        // (a GUI branches on `ok`), with error kind `unsupported` + the feature.
        let e = unsupported_envelope("dupes", "dupes apply", "not on Windows");
        assert!(e.contains("\"ok\":false"), "must be a failure: {e}");
        assert!(e.contains("\"kind\":\"unsupported\""), "got: {e}");
        assert!(e.contains("\"feature\":\"dupes apply\""), "got: {e}");
        assert!(e.contains("\"command\":\"dupes\""), "got: {e}");
        assert!(e.contains("\"message\":\"not on Windows\""), "got: {e}");
    }
}
