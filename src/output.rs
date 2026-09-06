//! Stable Burrow output envelope.
//!
//! Every conductor command emits a versioned envelope so agents get a consistent shape
//! regardless of which engine served the request. v0 embeds the engine's already-valid JSON
//! verbatim as `data`; non-JSON engine output is wrapped as `{"text": "..."}`.

use serde_json::Value;

/// Wrap engine output in the Burrow envelope, auto-detecting JSON vs plain text.
pub fn wrap(command: &str, engine: &str, engine_out: &str) -> String {
    let t = engine_out.trim();
    if serde_json::from_str::<Value>(t).is_ok() {
        envelope(command, engine, t)
    } else {
        envelope_text(command, engine, t)
    }
}

/// Wrap already-valid engine JSON in the Burrow envelope (`data` = the JSON, verbatim).
/// `engine` names what actually served the request (burrow-engine | fclones | czkawka |
/// bcu | native) — the field was previously hardcoded to burrow-engine.
///
/// Every envelope in this module takes it, success and failure alike, and every caller resolves
/// it through the one `engine_for` in `main.rs` — see [`error_envelope`] for what the failure
/// envelopes' own hardcoded copy cost.
pub fn envelope(command: &str, engine: &str, engine_json: &str) -> String {
    format!(
        "{{\"ok\":true,\"burrow_cli\":\"{}\",\"engine\":{},\"command\":{},\"data\":{}}}",
        env!("CARGO_PKG_VERSION"),
        json_string(engine),
        json_string(command),
        engine_json.trim()
    )
}

/// Wrap arbitrary engine text (e.g. a bash dry-run report) as `data.text`.
pub fn envelope_text(command: &str, engine: &str, text: &str) -> String {
    format!(
        "{{\"ok\":true,\"burrow_cli\":\"{}\",\"engine\":{},\"command\":{},\"data\":{{\"text\":{}}}}}",
        env!("CARGO_PKG_VERSION"),
        json_string(engine),
        json_string(command),
        json_string(text)
    )
}

/// The machine-readable classification the envelope's `error.kind` carries.
///
/// A GUI or agent branches on this — prompt for permissions, say "not available here", show a
/// generic failure — so it has to be a claim the producer can stand behind. It used to be GUESSED
/// from the message: a wording table in this module searched every conductor-raised failure for
/// substrings ("not found", "exited", "access is denied") and stamped whatever matched first. That
/// made the classification a side effect of prose — reword a message in `engine.rs` and `gui`'s
/// "not installed" quietly stopped being `not_found` — and it agreed with the engine's own
/// classification only by luck. Now the site that RAISES a failure names its kind, in code, and
/// the message is free text with no load-bearing words in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// The OS refused: `EACCES`, `ERROR_ACCESS_DENIED`, a UAC/elevation demand.
    PermissionDenied,
    /// Something named does not exist: an engine or sidecar that could not be resolved, a path
    /// the OS reports missing.
    NotFound,
    /// The capability exists somewhere, not here: a platform refusal.
    Unsupported,
    /// A process ran and produced output this conductor cannot read as what it should be.
    InvalidOutput,
    /// A process ran and exited non-zero for a reason it did not classify.
    ProcessFailed,
    /// The generic failure: malformed arguments, a parse error, anything with no sharper kind.
    Error,
    /// A kind RELAYED from an engine that this conductor has not been taught, carried verbatim.
    /// A caller's `match` falls into its default arm — honest — where substituting a kind the
    /// conductor recognizes would send it down a branch the engine never asked for.
    Other(String),
}

impl ErrorKind {
    /// The wire spelling, which is what the engine and every consumer share.
    pub fn as_str(&self) -> &str {
        match self {
            Self::PermissionDenied => "permission_denied",
            Self::NotFound => "not_found",
            Self::Unsupported => "unsupported",
            Self::InvalidOutput => "invalid_output",
            Self::ProcessFailed => "process_failed",
            Self::Error => "error",
            Self::Other(kind) => kind,
        }
    }

    /// A kind read off an engine envelope. The spellings this conductor knows map onto their
    /// variants; anything else is [`ErrorKind::Other`] and round-trips unchanged.
    pub fn from_wire(kind: &str) -> Self {
        match kind {
            "permission_denied" => Self::PermissionDenied,
            "not_found" => Self::NotFound,
            "unsupported" => Self::Unsupported,
            "invalid_output" => Self::InvalidOutput,
            "process_failed" => Self::ProcessFailed,
            "error" => Self::Error,
            other => Self::Other(other.to_string()),
        }
    }

    /// The kind an OS error is, read from its structured `io::ErrorKind` rather than its text.
    pub fn from_io(e: &std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::NotFound => Self::NotFound,
            std::io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            _ => Self::Error,
        }
    }
}

/// A failure on its way into the error envelope: the reason, its classification, and — for a
/// failure that ran a process — the narrowly-scoped details of what ran.
///
/// There is no `Default` and no bare struct literal on purpose: whoever raises a failure has to
/// say what kind it is. `From<String>`/`From<&str>` exist for the plain argument-shaped refusals
/// (`needs a directory`) and give the generic [`ErrorKind::Error`], which is the one kind a bare
/// sentence can honestly claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The human-readable reason, verbatim, whoever produced it.
    pub message: String,
    kind: ErrorKind,
    /// Machine-readable process details (`program`, `args`, `exit_code`, `stdout`, `stderr`,
    /// plus `engine` and `applied`), nested under `error.details` when present.
    details: Option<Value>,
}

impl Failure {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind,
            details: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Error, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }

    pub fn invalid_output(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidOutput, message)
    }

    pub fn process_failed(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::ProcessFailed, message)
    }

    /// An OS error in `context` ("cannot read plan x"), classified from the error's own
    /// `io::ErrorKind` — a missing file is `not_found`, a refused one `permission_denied`.
    pub fn io(context: impl std::fmt::Display, e: &std::io::Error) -> Self {
        Self::new(ErrorKind::from_io(e), format!("{context}: {e}"))
    }

    /// A failure RELAYED from an engine's own envelope, carrying the engine's `kind` verbatim.
    ///
    /// Relaying is deliberately all-or-nothing (see `engine::engine_failure`): the message and
    /// the kind are one classified answer, and pairing the engine's kind with a message the
    /// conductor wrote would attribute a classification to text it does not describe.
    pub fn relayed(message: impl Into<String>, kind: &str) -> Self {
        Self::new(ErrorKind::from_wire(kind), message)
    }

    /// Attach process details, emitted under `error.details`.
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    /// The classification. Test-only: production hands the whole `Failure` to
    /// [`error_envelope`] and never branches on the kind anywhere else.
    #[cfg(test)]
    pub fn kind(&self) -> &ErrorKind {
        &self.kind
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::error(message)
    }
}

impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Self::error(message)
    }
}

/// The structured error payload shared by the error/unsupported envelopes:
/// `{ "kind": …, "message": …, "platform": … }`.
fn error_object(kind: &str, message: &str, details: Option<&Value>) -> String {
    match details {
        Some(details) => format!(
            "{{\"kind\":{},\"message\":{},\"platform\":\"{}\",\"details\":{}}}",
            json_string(kind),
            json_string(message),
            std::env::consts::OS,
            details
        ),
        None => format!(
            "{{\"kind\":{},\"message\":{},\"platform\":\"{}\"}}",
            json_string(kind),
            json_string(message),
            std::env::consts::OS
        ),
    }
}

/// Wrap a failure — top-level `ok:false` plus a structured `error {kind, message,
/// platform[, details]}` — so a GUI/agent parses ONE shape, branches on `ok`, and gets a
/// classified reason. (Combines #4's ok-branching with #5's error classification.)
///
/// `burrow_cli` names the CONDUCTOR here, on the relayed path as much as anywhere else. A
/// relayed failure came out of an engine envelope carrying the ENGINE's version in that field;
/// the conductor re-emits rather than forwarding precisely so consumers scraping a version out
/// of it get the one that answered them. The engine's `kind`, `message`, and optional `details`
/// cross over together.
///
/// # `engine` answers the same question here as it does on the success envelope
///
/// It was hardcoded `burrow-engine` on this path, so every command the conductor serves ITSELF
/// named one engine when it worked and a different one when it did not. `win-uninstall` is the
/// clearest form of it: the success envelope says `bcu` (via `engine_for`), and a BCU that could
/// not be resolved came back `burrow-engine` at the top level while the `error.details` this same
/// call site attaches said `"engine":"bcu"` — one envelope disagreeing with itself. A caller that
/// routes, filters, or reports on `engine` cannot use a value that flips with the outcome.
///
/// So the caller passes it, resolved through the same `engine_for` that feeds `emit`, and the
/// answer means one thing throughout: WHO SERVES THIS COMMAND. Who *refused* is a separate
/// question with its own separate marker — see [`unsupported_envelope`].
pub fn error_envelope(command: &str, engine: &str, failure: &Failure) -> String {
    format!(
        "{{\"ok\":false,\"burrow_cli\":\"{}\",\"engine\":{},\"command\":{},\"error\":{}}}",
        env!("CARGO_PKG_VERSION"),
        json_string(engine),
        json_string(command),
        error_object(
            failure.kind.as_str(),
            &failure.message,
            failure.details.as_ref()
        )
    )
}

/// A platform-unsupported failure — a failure envelope whose error `kind` is
/// `unsupported`, plus the `feature` that's unavailable on this platform.
///
/// This is the CONDUCTOR's own refusal, raised before any engine runs, and the top-level
/// `feature` key is what says so. An engine `unsupported` envelope carries a `feature` too, and
/// [`error_envelope`] deliberately does NOT relay it: presence of the key is documented in
/// `README.md` as how a caller tells a conductor-side refusal from a relayed engine one, and
/// relaying it would collapse the two into one indistinguishable shape. `error.kind` answers a
/// different question — WHY the call failed — so relaying that costs the distinction nothing.
///
/// `engine` is a third question again, and the reason this takes it rather than naming the
/// conductor: it says who SERVES the command, not who answered this particular call. Hardcoding
/// it split three of the four refusals off from the rest of their own command's envelopes —
/// `slim` and `gui` said `burrow-engine` where every other envelope of theirs says `native`, and
/// `win-dupes --apply` said it where every other `win-dupes` envelope says `czkawka`. Only
/// `sentinel --watch` agreed, and only by accident: the engine really does serve the other half
/// of `sentinel`. Threading it pairs all four, and `feature` goes on carrying the "this refusal
/// is the conductor's own" half by itself, which is the job `README.md` already gives it.
pub fn unsupported_envelope(command: &str, engine: &str, feature: &str, detail: &str) -> String {
    format!(
        "{{\"ok\":false,\"burrow_cli\":\"{}\",\"engine\":{},\"command\":{},\"error\":{},\"feature\":{}}}",
        env!("CARGO_PKG_VERSION"),
        json_string(engine),
        json_string(command),
        error_object(ErrorKind::Unsupported.as_str(), detail, None),
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

    /// A REAL `burrow-engine` failure envelope, captured verbatim off a release build whose `net`
    /// message was reworded — see `testdata/engine-captures/PROVENANCE.txt` for exactly how. Its
    /// message contains none of the words the retired wording table used to look for, so it is
    /// the capture that proves the kind comes off the wire and not off the prose.
    const REWORDED: &str =
        include_str!("../testdata/engine-captures/net-unsupported-reworded.json");

    /// Who serves the captured commands. `main.rs`'s `engine_for` is what production resolves
    /// this from; these tests are about `kind` relaying and only need the attribution to be the
    /// one those commands really carry, so a caller reading them is not shown a false pairing.
    const ENGINE: &str = "burrow-engine";

    /// `(command, message, kind)` from a captured engine envelope.
    fn captured(capture: &str) -> (String, String, String) {
        let v: Value = serde_json::from_str(capture).expect("capture must be valid JSON");
        let s = |p: &Value| {
            p.as_str()
                .expect("captured field must be a string")
                .to_string()
        };
        (
            s(&v["command"]),
            s(&v["error"]["message"]),
            s(&v["error"]["kind"]),
        )
    }

    fn error_field(envelope: &str, field: &str) -> String {
        let v: Value = serde_json::from_str(envelope).expect("envelope must be valid JSON");
        v["error"][field]
            .as_str()
            .unwrap_or_else(|| panic!("envelope must carry error.{field}: {envelope}"))
            .to_string()
    }

    /// The engine's classification reaches the caller with the message it classifies, and the
    /// message carries no words that could have produced the kind on their own.
    #[test]
    fn a_relayed_engine_kind_survives_the_conductor() {
        let (command, message, engine_kind) = captured(REWORDED);
        let relayed = error_envelope(&command, ENGINE, &Failure::relayed(&message, &engine_kind));
        assert_eq!(
            error_field(&relayed, "kind"),
            engine_kind,
            "the engine's own classification must survive the conductor: {relayed}"
        );
        assert_eq!(
            error_field(&relayed, "message"),
            message,
            "the message it classifies must survive with it: {relayed}"
        );
    }

    /// The relay must not quietly become "always `unsupported`". `slim-check` is the capture
    /// where the engine itself says `error`, and relaying has to report exactly that.
    #[test]
    fn a_relayed_kind_is_whatever_the_engine_said_including_the_generic_one() {
        let capture = include_str!("../testdata/engine-captures/slim-check-error.json");
        let (command, message, engine_kind) = captured(capture);
        let relayed = error_envelope(&command, ENGINE, &Failure::relayed(&message, &engine_kind));
        assert_eq!(error_field(&relayed, "kind"), engine_kind, "{relayed}");
    }

    /// A kind the conductor has never seen is passed through UNCHANGED rather than normalized
    /// into one it recognizes. A caller's `match` falls into its default arm — honest — where a
    /// substituted kind would send it down a branch the engine never asked for.
    #[test]
    fn an_unrecognized_engine_kind_is_relayed_verbatim() {
        let (command, message, _) = captured(REWORDED);
        let future_kind = "quarantined_by_gatekeeper";
        let relayed = error_envelope(&command, ENGINE, &Failure::relayed(&message, future_kind));
        assert_eq!(
            error_field(&relayed, "kind"),
            future_kind,
            "an unknown kind must reach the caller as-is: {relayed}"
        );
        assert_eq!(
            ErrorKind::from_wire(future_kind),
            ErrorKind::Other(future_kind.to_string())
        );
    }

    /// Every spelling this conductor knows round-trips through the wire form, so a relayed
    /// `not_found` is the same variant a conductor-raised `not_found` is.
    #[test]
    fn known_kinds_round_trip_through_their_wire_spelling() {
        for kind in [
            ErrorKind::PermissionDenied,
            ErrorKind::NotFound,
            ErrorKind::Unsupported,
            ErrorKind::InvalidOutput,
            ErrorKind::ProcessFailed,
            ErrorKind::Error,
        ] {
            assert_eq!(ErrorKind::from_wire(kind.as_str()), kind);
        }
    }

    /// The other half of the contract: a failure NO engine classified carries the kind its
    /// raising site declared — and ONLY that. Same message as the relay test above, raised as a
    /// plain conductor error, so the two differ only in who classified it: the engine's
    /// `unsupported` must not appear, because nothing relayed it.
    #[test]
    fn a_conductor_raised_failure_carries_the_kind_its_site_declared() {
        let (command, message, engine_kind) = captured(REWORDED);
        let raised = error_envelope(&command, ENGINE, &Failure::error(&message));
        assert_eq!(error_field(&raised, "kind"), "error", "{raised}");
        assert_ne!(
            error_field(&raised, "kind"),
            engine_kind,
            "nothing relayed here, so the engine's kind must not appear: {raised}"
        );
    }

    /// The message is free text: no word in it changes the kind. The retired wording table would
    /// have read "not found" out of this sentence and answered `not_found`.
    #[test]
    fn the_message_never_decides_the_kind() {
        let e = error_envelope(
            "clean",
            ENGINE,
            &Failure::error("engine \"mole\" not found — access is denied and it exited"),
        );
        assert!(e.contains("\"kind\":\"error\""), "got: {e}");
    }

    /// An OS error is classified from its structured `io::ErrorKind`, not its text.
    #[test]
    fn io_errors_classify_from_their_kind() {
        use std::io::{Error, ErrorKind as Io};
        let missing = Failure::io("cannot read plan x", &Error::from(Io::NotFound));
        assert_eq!(missing.kind(), &ErrorKind::NotFound);
        assert!(
            missing.message.starts_with("cannot read plan x: "),
            "{missing:?}"
        );
        let denied = Failure::io("write y", &Error::from(Io::PermissionDenied));
        assert_eq!(denied.kind(), &ErrorKind::PermissionDenied);
        let other = Failure::io("z", &Error::from(Io::Other));
        assert_eq!(other.kind(), &ErrorKind::Error);
    }

    /// `burrow_cli` names the CONDUCTOR even when the failure came out of an engine envelope that
    /// carried the engine's own version there. Consumers scrape a version out of this field; the
    /// relay moves `error.kind` across and nothing else.
    #[test]
    fn a_relayed_failure_still_reports_the_conductors_version() {
        let (command, message, engine_kind) = captured(REWORDED);
        let captured_version: Value = serde_json::from_str(REWORDED).unwrap();
        let engine_version = captured_version["burrow_cli"].as_str().unwrap();

        let relayed = error_envelope(&command, ENGINE, &Failure::relayed(&message, &engine_kind));
        let v: Value = serde_json::from_str(&relayed).unwrap();
        assert_eq!(
            v["burrow_cli"],
            serde_json::json!(env!("CARGO_PKG_VERSION"))
        );
        assert_ne!(
            v["burrow_cli"],
            serde_json::json!(engine_version),
            "the engine's version must not leak into the conductor's field: {relayed}"
        );
    }

    /// The engine's top-level `feature` key does NOT cross over. `README.md` documents its
    /// presence as how a caller tells a conductor-side refusal from a relayed engine one, and
    /// relaying it would collapse two distinguishable shapes into one.
    #[test]
    fn a_relayed_failure_does_not_carry_the_engines_feature_key() {
        let (command, message, engine_kind) = captured(REWORDED);
        let capture: Value = serde_json::from_str(REWORDED).unwrap();
        assert!(
            capture.get("feature").is_some(),
            "the capture must actually carry a feature key or this test proves nothing"
        );

        let relayed = error_envelope(&command, ENGINE, &Failure::relayed(&message, &engine_kind));
        let v: Value = serde_json::from_str(&relayed).unwrap();
        assert!(
            v.get("feature").is_none(),
            "only the conductor's own refusal marks a feature: {relayed}"
        );
    }

    #[test]
    fn envelope_wraps_object() {
        let e = envelope("status", "burrow-engine", "{\"a\":1}");
        assert!(e.starts_with('{') && e.ends_with('}'));
        assert!(e.contains("\"command\":\"status\""));
        assert!(e.contains("\"engine\":\"burrow-engine\""));
        assert!(e.contains("\"data\":{\"a\":1}"));
    }

    #[test]
    fn wrap_detects_array() {
        assert!(wrap("status", "burrow-engine", " [1,2] ").contains("\"data\":[1,2]"));
    }

    #[test]
    fn wrap_text_is_escaped_json() {
        let e = wrap(
            "clean",
            "burrow-engine",
            "Would remove:\n\t\"~/Library/Caches\"",
        );
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
        let e = envelope("status", "burrow-engine", "{\"a\":1}");
        assert!(
            e.contains("\"ok\":true"),
            "success envelope must carry ok:true, got: {e}"
        );
    }

    #[test]
    fn error_envelope_is_a_classified_failure() {
        // Failure = top-level ok:false + a structured error {kind, message, platform}.
        let e = error_envelope(
            "clean",
            ENGINE,
            &Failure::not_found("engine \"mole\" not found"),
        );
        assert!(e.contains("\"ok\":false"), "got: {e}");
        assert!(e.contains("\"command\":\"clean\""), "got: {e}");
        assert!(e.contains("\"kind\":\"not_found\""), "classified: {e}");
        // the message is a valid, escaped JSON string nested under `error`.
        assert!(
            e.contains("\"message\":\"engine \\\"mole\\\" not found\""),
            "got: {e}"
        );
        assert!(e.contains("\"platform\":"), "got: {e}");
        assert!(
            !e.contains("\"details\""),
            "no details were attached, so none are emitted: {e}"
        );
    }

    #[test]
    fn every_kind_has_its_wire_spelling_in_the_envelope() {
        for (kind, wire) in [
            (ErrorKind::PermissionDenied, "permission_denied"),
            (ErrorKind::InvalidOutput, "invalid_output"),
            (ErrorKind::ProcessFailed, "process_failed"),
            (ErrorKind::Unsupported, "unsupported"),
        ] {
            let e = error_envelope("win-dupes", "czkawka", &Failure::new(kind, "m"));
            assert!(e.contains(&format!("\"kind\":\"{wire}\"")), "got: {e}");
        }
    }

    #[test]
    fn failure_details_are_nested_under_error() {
        let details = serde_json::json!({
            "program": "BCU-console.exe",
            "args": ["uninstall", "Foo"],
            "exit_code": 5,
        });
        let e = error_envelope(
            "win-uninstall",
            "bcu",
            &Failure::new(
                ErrorKind::PermissionDenied,
                "BCU exited 5: Access is denied",
            )
            .with_details(details.clone()),
        );
        let parsed: Value = serde_json::from_str(&e).unwrap();
        assert_eq!(parsed["ok"], serde_json::json!(false));
        assert_eq!(
            parsed["error"]["kind"],
            serde_json::json!("permission_denied")
        );
        assert_eq!(parsed["error"]["details"], details);
    }

    #[test]
    fn unsupported_envelope_is_a_failure_not_success() {
        // Regression: a platform-unsupported response must be top-level ok:false
        // (a GUI branches on `ok`), with error kind `unsupported` + the feature.
        let e = unsupported_envelope("dupes", ENGINE, "dupes apply", "not on Windows");
        assert!(e.contains("\"ok\":false"), "must be a failure: {e}");
        assert!(e.contains("\"kind\":\"unsupported\""), "got: {e}");
        assert!(e.contains("\"feature\":\"dupes apply\""), "got: {e}");
        assert!(e.contains("\"command\":\"dupes\""), "got: {e}");
        assert!(e.contains("\"message\":\"not on Windows\""), "got: {e}");
    }
}
