//! `burrow.rules/v1` — declarative, agent-readable per-app cleaning rules.
//!
//! Design (landscape doc 1.2): per-app rules keyed by bundle id, a CLOSED action enum (no
//! arbitrary shell — every rule is statically auditable and dry-runnable), risk tiers that
//! drive default selection, trash-vs-remove semantics, and a required `provenance` block with
//! evidence citations (the agent-native differentiator). The format is JSON for agent-nativeness.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuleFile {
    pub schema: String,
    pub app: AppSpec,
    #[serde(default)]
    pub rules: Vec<Rule>,
    pub provenance: Provenance,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppSpec {
    pub bundle_ids: Vec<String>,
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub category: String,
    pub risk: Risk,
    #[serde(default)]
    pub recommend: bool,
    #[serde(default)]
    pub explain: Option<String>,
    /// Per-rule opt-in for AUTOMATED cleaning (plan Phase 8): only `auto: true` rules may
    /// be planned by `trigger --rules`. Default false; validation restricts it to risk:safe.
    #[serde(default)]
    pub auto: bool,
    pub targets: Vec<Target>,
    pub action: Action,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    Safe,
    Caution,
    Risky,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Search {
    File,
    Glob,
    WalkFiles,
    #[default]
    WalkAll,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub path: String,
    #[serde(default)]
    pub search: Search,
    /// Named conditions gating selection (plan Phase 4): every present condition must hold
    /// or the target is not auto-selected. Absent = unconditional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Conditions>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Conditions {
    /// Only select entries at least this many days old (by modification time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_age_days: Option<u64>,
    /// Only select entries at least this many bytes (files only; unverifiable = not met).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_size_bytes: Option<u64>,
    /// Only select entries unaccessed for at least this many days (by access time — best
    /// effort; noatime mounts can make recently read data look old, so an atime condition
    /// is not proof that a file is unused).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_days_unaccessed: Option<u64>,
}

/// What [`probe_path`] could measure about a path, for [`condition_met`]. Each field is `None`
/// when the metadata was unavailable — and an unavailable measurement never satisfies a
/// condition, so an unprobeable path is never auto-selected.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ProbeStats {
    /// Days since last modification.
    pub age_days: Option<u64>,
    /// Size in bytes — files only; directories report `None` (a recursive walk would be
    /// dryrun-hostile), so size conditions on directories stay unmet.
    pub size_bytes: Option<u64>,
    /// Days since last access (best effort — see [`Conditions::min_days_unaccessed`]).
    pub unaccessed_days: Option<u64>,
}

/// Pure condition check. CONSERVATIVE: a condition whose metadata is unavailable is NOT met
/// — we never auto-select what we can't verify.
pub fn condition_met(c: &Conditions, stats: &ProbeStats) -> bool {
    fn holds(min: Option<u64>, actual: Option<u64>) -> bool {
        match (min, actual) {
            (None, _) => true,
            (Some(m), Some(a)) => a >= m,
            (Some(_), None) => false,
        }
    }
    holds(c.min_age_days, stats.age_days)
        && holds(c.min_size_bytes, stats.size_bytes)
        && holds(c.min_days_unaccessed, stats.unaccessed_days)
}

/// Probe a path's [`ProbeStats`] for condition evaluation.
pub fn probe_path(p: &Path) -> ProbeStats {
    let Ok(md) = std::fs::metadata(p) else {
        return ProbeStats::default();
    };
    let days_since = |t: std::io::Result<std::time::SystemTime>| {
        t.ok()
            .and_then(|m| m.elapsed().ok())
            .map(|e| e.as_secs() / 86_400)
    };
    ProbeStats {
        age_days: days_since(md.modified()),
        size_bytes: md.is_file().then_some(md.len()),
        unaccessed_days: days_since(md.accessed()),
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    /// Closed enum — never arbitrary shell, so every rule is dry-runnable + size-estimable.
    #[serde(rename = "type")]
    pub kind: ActionType,
    #[serde(default)]
    pub method: Method,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ActionType {
    Delete,
    Truncate,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    #[default]
    Trash,
    Remove,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    /// builtin | community | agent | user
    pub source: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub license: Option<String>,
}

/// Parse a rule file from JSON.
pub fn parse(json: &str) -> Result<RuleFile, String> {
    serde_json::from_str(json).map_err(|e| format!("rule parse error: {e}"))
}

/// Validate a rule file; returns human-readable problems (empty = valid).
pub fn validate(rf: &RuleFile) -> Vec<String> {
    let mut errs = Vec::new();
    if rf.schema != "burrow.rules/v1" {
        errs.push(format!(
            "schema must be 'burrow.rules/v1', got '{}'",
            rf.schema
        ));
    }
    if rf.app.bundle_ids.is_empty() {
        errs.push("app.bundle_ids must not be empty".into());
    }
    if rf.provenance.source.trim().is_empty() {
        errs.push("provenance.source is required".into());
    }
    for r in &rf.rules {
        if r.targets.is_empty() {
            errs.push(format!("rule '{}' has no targets", r.id));
        }
        if r.risk == Risk::Risky && r.recommend {
            errs.push(format!(
                "rule '{}' is risky but recommend=true (risky must never be preselected)",
                r.id
            ));
        }
        if r.auto && r.risk != Risk::Safe {
            errs.push(format!(
                "rule '{}' is auto but not risk:safe (automation may only touch safe rules)",
                r.id
            ));
        }
        if r.auto && (r.action.kind != ActionType::Delete || r.action.method != Method::Trash) {
            errs.push(format!(
                "rule '{}' cannot be automated: trigger plans only delete-to-trash actions",
                r.id
            ));
        }
        for t in &r.targets {
            if t.path.trim().is_empty() || t.path.contains('\0') {
                errs.push(format!("rule '{}' has an invalid target path", r.id));
            }
            if r.auto && matches!(t.search, Search::Glob | Search::WalkFiles) {
                errs.push(format!(
                    "rule '{}' cannot be automated: this target search mode is not implemented",
                    r.id
                ));
            }
            if let Some(w) = &t.when {
                if w.min_age_days.is_none()
                    && w.min_size_bytes.is_none()
                    && w.min_days_unaccessed.is_none()
                {
                    errs.push(format!(
                        "rule '{}' target '{}' has an empty when{{}} (name at least one condition)",
                        r.id, t.path
                    ));
                }
            }
        }
    }
    errs
}

/// Whether a rule is preselected in quick-clean. Only `safe` + `recommend`; `risky` never.
pub fn default_selected(r: &Rule) -> bool {
    r.risk == Risk::Safe && r.recommend
}

/// Expand a leading `~` against `home`. (Env-var expansion is a later addition.)
pub fn expand_path(path: &str, home: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else if path == "~" {
        home.to_string()
    } else {
        path.to_string()
    }
}

/// A rule file loaded from disk (parse may have failed).
pub struct Loaded {
    pub file: String,
    pub result: Result<RuleFile, String>,
}

/// Load every `*.json` rule file in a directory (sorted by name).
///
/// A failing directory ENTRY fails the whole load, rather than being skipped. This used to be a
/// `filter_map(|e| e.ok())`, which made an unreadable entry vanish — and the caller that matters
/// is `auto_clean_plan`, which turns these files into a list of paths something will DELETE. A
/// silently dropped rule file there is indistinguishable from a rule file that matched nothing,
/// which is the same failure mode this module already refused to accept for parse errors (those
/// are reported per-file through `Loaded::result`). A directory this function cannot enumerate in
/// full is one it cannot honestly answer about, so it says so.
pub fn load_dir(dir: &Path) -> Result<Vec<Loaded>, String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|e| format!("cannot read an entry of {}: {e}", dir.display()))?
            .path();
        if path.extension().is_some_and(|x| x == "json") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths
        .into_iter()
        .map(|p| {
            let file = p.file_name().unwrap().to_string_lossy().into_owned();
            let result = std::fs::read_to_string(&p)
                .map_err(|e| format!("read error: {e}"))
                .and_then(|s| parse(&s));
            Loaded { file, result }
        })
        .collect())
}

/// The auto-clean plan: existing target paths of `auto: true` rules that are default-
/// selected (safe + recommend) with conditions met. Everything else is excluded — the
/// per-rule opt-in is the whole point. `home` expands a leading `~` in rule paths.
///
/// Returns the plan and the names of any rule files it refused to plan from.
///
/// # Why this validates, when it used to just parse
///
/// This path turns rule files into a list of paths something will DELETE, and it used to skip
/// only the files that failed to parse — a file that deserialized but was semantically wrong
/// (a schema version this build does not implement, an app with no bundle ids) still contributed
/// targets. That gap was survivable while `burrow rules validate` existed here to catch it, and
/// CI validated the shipped seed rules through it. That surface is the engine's now, so the
/// conductor would be planning deletions from files it has no way to check. It checks them here
/// instead, which is the better place anyway: the check now runs on the path that acts, rather
/// than in a command a user has to remember to run.
///
/// Skips are REPORTED rather than swallowed. A malformed rule file otherwise shows up as
/// auto-clean quietly planning nothing, which looks identical to "nothing needed cleaning".
pub fn auto_clean_plan(rules_dir: &Path, home: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let mut plan = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut skipped = Vec::new();
    for l in load_dir(rules_dir)? {
        let Ok(rf) = l.result else {
            skipped.push(l.file);
            continue;
        };
        if !validate(&rf).is_empty() {
            skipped.push(l.file);
            continue;
        }
        for r in &rf.rules {
            if !r.auto || !default_selected(r) {
                continue;
            }
            for t in &r.targets {
                let p = expand_path(&t.path, home);
                let path = Path::new(&p);
                // Held plans may run from another working directory. Never turn a relative
                // path or the home/root directory itself into an unattended trash operation.
                if !path.is_absolute() || path.parent().is_none() || path == Path::new(home) {
                    if !skipped.contains(&l.file) {
                        skipped.push(l.file.clone());
                    }
                    continue;
                }
                if !path.exists() {
                    continue;
                }
                if (t.search == Search::File && !path.is_file())
                    || (t.when.is_some() && path.is_dir())
                {
                    // Directory timestamps do not establish the age of the files below them.
                    // Until recursive conditions are implemented this target cannot be verified.
                    if !skipped.contains(&l.file) {
                        skipped.push(l.file.clone());
                    }
                    continue;
                }
                let met = t
                    .when
                    .as_ref()
                    .is_none_or(|w| condition_met(w, &probe_path(path)));
                if met && seen.insert(p.clone()) {
                    plan.push(p);
                }
            }
        }
    }
    Ok((plan, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(age: Option<u64>, size: Option<u64>, unaccessed: Option<u64>) -> ProbeStats {
        ProbeStats {
            age_days: age,
            size_bytes: size,
            unaccessed_days: unaccessed,
        }
    }

    /// Only the auto-opted, default-selected rule's EXISTING targets are planned; a file that
    /// does not validate is named as skipped rather than silently contributing nothing.
    #[test]
    fn auto_clean_plan_selects_auto_rules_and_names_skipped_files() {
        let dir = std::env::temp_dir().join(format!("burrow_autoplan_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let auto_target = dir.join("auto_cache");
        let manual_target = dir.join("manual_cache");
        std::fs::write(&auto_target, "x").unwrap();
        std::fs::write(&manual_target, "x").unwrap();
        let good = serde_json::json!({
            "schema": "burrow.rules/v1",
            "app": { "bundle_ids": ["com.x.y"], "name": "X" },
            "rules": [
                {"id": "x.auto", "category": "cache", "risk": "safe", "recommend": true,
                 "auto": true, "targets": [{ "path": auto_target.to_string_lossy() }],
                 "action": { "type": "delete" }},
                {"id": "x.manual", "category": "cache", "risk": "safe", "recommend": true,
                 "targets": [{ "path": manual_target.to_string_lossy() }],
                 "action": { "type": "delete" }},
                {"id": "x.missing", "category": "cache", "risk": "safe", "recommend": true,
                 "auto": true, "targets": [{ "path": dir.join("absent").to_string_lossy() }],
                 "action": { "type": "delete" }}
            ],
            "provenance": { "source": "builtin" }
        });
        std::fs::write(dir.join("a.json"), good.to_string()).unwrap();
        // Parses, but fails validation (no bundle ids) — must be skipped by name.
        let invalid = good
            .to_string()
            .replace(r#""bundle_ids":["com.x.y"]"#, r#""bundle_ids":[]"#);
        assert_ne!(invalid, good.to_string());
        std::fs::write(dir.join("b.json"), invalid).unwrap();
        std::fs::write(dir.join("c.json"), "not json").unwrap();

        let (plan, skipped) = auto_clean_plan(&dir, "/home/x").unwrap();
        assert_eq!(plan, vec![auto_target.to_string_lossy().into_owned()]);
        assert_eq!(skipped, vec!["b.json".to_string(), "c.json".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn auto_clean_plan_reports_an_unreadable_directory() {
        let missing =
            std::env::temp_dir().join(format!("burrow_autoplan_missing_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&missing);
        assert!(auto_clean_plan(&missing, "/home/x").is_err());
    }

    // --- named conditions (plan Phase 4: min_age_days / min_size_bytes) ---

    #[test]
    fn target_when_conditions_parse() {
        let json = r#"{
            "schema": "burrow.rules/v1",
            "app": { "bundle_ids": ["com.x.y"], "name": "X" },
            "rules": [{
                "id": "x.old-caches", "category": "cache", "risk": "safe",
                "recommend": true, "explain": "e",
                "targets": [{ "path": "~/t", "when": { "min_age_days": 30, "min_size_bytes": 1024 } }],
                "action": { "type": "delete", "method": "trash" }
            }],
            "provenance": { "source": "builtin" }
        }"#;
        let rf = parse(json).unwrap();
        let when = rf.rules[0].targets[0].when.as_ref().unwrap();
        assert_eq!(when.min_age_days, Some(30));
        assert_eq!(when.min_size_bytes, Some(1024));
        assert!(validate(&rf).is_empty(), "{:?}", validate(&rf));
    }

    #[test]
    fn condition_met_requires_every_present_condition() {
        let both = Conditions {
            min_age_days: Some(30),
            min_size_bytes: Some(1024),
            ..Default::default()
        };
        assert!(condition_met(&both, &stats(Some(40), Some(2048), None)));
        assert!(
            !condition_met(&both, &stats(Some(10), Some(2048), None)),
            "too young"
        );
        assert!(
            !condition_met(&both, &stats(Some(40), Some(10), None)),
            "too small"
        );
        let age_only = Conditions {
            min_age_days: Some(30),
            ..Default::default()
        };
        assert!(
            condition_met(&age_only, &stats(Some(31), None, None)),
            "size not required"
        );
    }

    #[test]
    fn condition_met_is_conservative_on_missing_metadata() {
        // Can't verify -> not met -> never auto-selected.
        let c = Conditions {
            min_age_days: Some(30),
            ..Default::default()
        };
        assert!(!condition_met(&c, &ProbeStats::default()));
    }

    #[test]
    fn min_days_unaccessed_gates_on_atime() {
        // Phase 8: `min_days_unaccessed` — untouched-for-N-days material only.
        let c = Conditions {
            min_days_unaccessed: Some(30),
            ..Default::default()
        };
        assert!(condition_met(&c, &stats(None, None, Some(45))));
        assert!(
            !condition_met(&c, &stats(None, None, Some(3))),
            "recently used"
        );
        assert!(
            !condition_met(&c, &ProbeStats::default()),
            "unverifiable atime"
        );
    }

    // --- per-rule automation opt-in (plan Phase 8: scheduled cleaning, per-rule opt-in) ---

    #[test]
    fn auto_defaults_off_and_parses() {
        let json = r#"{
            "schema": "burrow.rules/v1",
            "app": { "bundle_ids": ["com.x.y"], "name": "X" },
            "rules": [
                {"id": "x.a", "category": "cache", "risk": "safe", "recommend": true,
                 "explain": "e", "auto": true,
                 "targets": [{ "path": "~/a" }], "action": { "type": "delete" }},
                {"id": "x.b", "category": "cache", "risk": "safe", "recommend": true,
                 "explain": "e",
                 "targets": [{ "path": "~/b" }], "action": { "type": "delete" }}
            ],
            "provenance": { "source": "builtin" }
        }"#;
        let rf = parse(json).unwrap();
        assert!(rf.rules[0].auto);
        assert!(!rf.rules[1].auto, "auto must default to false (opt-in)");
        assert!(validate(&rf).is_empty());
    }

    #[test]
    fn validate_rejects_auto_on_non_safe_rules() {
        // Automation may only ever touch risk:safe rules — auto-cleaning caution/risky
        // material defeats explain-before-delete.
        let json = r#"{
            "schema": "burrow.rules/v1",
            "app": { "bundle_ids": ["com.x.y"], "name": "X" },
            "rules": [{"id": "x.r", "category": "cache", "risk": "caution",
                "recommend": false, "explain": "e", "auto": true,
                "targets": [{ "path": "~/t" }], "action": { "type": "delete" }}],
            "provenance": { "source": "builtin" }
        }"#;
        let rf = parse(json).unwrap();
        let errs = validate(&rf);
        assert!(
            errs.iter().any(|e| e.contains("auto")),
            "auto on non-safe must be a validation problem: {errs:?}"
        );
    }

    #[test]
    fn validate_rejects_empty_when_block() {
        let json = r#"{
            "schema": "burrow.rules/v1",
            "app": { "bundle_ids": ["com.x.y"], "name": "X" },
            "rules": [{
                "id": "x.r", "category": "cache", "risk": "safe",
                "recommend": true, "explain": "e",
                "targets": [{ "path": "~/t", "when": {} }],
                "action": { "type": "delete" }
            }],
            "provenance": { "source": "builtin" }
        }"#;
        let rf = parse(json).unwrap();
        let errs = validate(&rf);
        assert!(
            errs.iter().any(|e| e.contains("when")),
            "empty when{{}} must be a validation problem: {errs:?}"
        );
    }

    const VALID: &str = r#"{
        "schema": "burrow.rules/v1",
        "app": { "bundle_ids": ["com.example.App"], "name": "Example" },
        "rules": [{
            "id": "example.cache", "category": "cache", "risk": "safe", "recommend": true,
            "targets": [{ "path": "~/Library/Caches/com.example.App" }],
            "action": { "type": "delete", "method": "trash" }
        }],
        "provenance": { "source": "builtin", "evidence": ["vendor docs"], "license": "Apache-2.0" }
    }"#;

    #[test]
    fn parses_valid_rule() {
        let rf = parse(VALID).unwrap();
        assert_eq!(rf.app.name, "Example");
        assert_eq!(rf.rules.len(), 1);
        assert_eq!(rf.rules[0].risk, Risk::Safe);
        // default search mode is walk_all
        assert_eq!(rf.rules[0].targets[0].search, Search::WalkAll);
        // default method is trash
        assert_eq!(rf.rules[0].action.method, Method::Trash);
    }

    #[test]
    fn valid_rule_has_no_problems() {
        assert!(validate(&parse(VALID).unwrap()).is_empty());
    }

    #[test]
    fn rejects_bad_schema() {
        let bad = VALID.replace("burrow.rules/v1", "burrow.rules/v0");
        let errs = validate(&parse(&bad).unwrap());
        assert!(errs.iter().any(|e| e.contains("schema")));
    }

    #[test]
    fn rejects_risky_recommend() {
        let bad = VALID.replace("\"risk\": \"safe\"", "\"risk\": \"risky\"");
        let errs = validate(&parse(&bad).unwrap());
        assert!(errs.iter().any(|e| e.contains("risky")), "errs: {errs:?}");
    }

    #[test]
    fn default_selection_rules() {
        let rf = parse(VALID).unwrap();
        assert!(default_selected(&rf.rules[0])); // safe + recommend
        let risky = parse(&VALID.replace("\"risk\": \"safe\"", "\"risk\": \"caution\"")).unwrap();
        assert!(!default_selected(&risky.rules[0])); // caution is not auto-selected
    }

    #[test]
    fn expands_tilde() {
        assert_eq!(
            expand_path("~/Library/Caches", "/Users/x"),
            "/Users/x/Library/Caches"
        );
        assert_eq!(expand_path("/abs/path", "/Users/x"), "/abs/path");
        assert_eq!(expand_path("~", "/Users/x"), "/Users/x");
    }

    /// The seed rule files this crate SHIPS must all parse and validate.
    ///
    /// This used to be an integration test driving `burrow rules validate`. That command is the
    /// engine's now, so checking the shipped files through the CLI would test the engine's
    /// validator against burrow-cli's assets — the wrong pairing, since it is THIS crate's
    /// `auto_clean_plan` that reads these files and turns them into deletion targets. So the
    /// check moved down here, onto the parser and validator that actually consume them.
    #[test]
    fn shipped_seed_rules_all_parse_and_validate() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rules");
        let loaded = load_dir(&dir).expect("the shipped rules/ directory must be readable");
        assert!(
            !loaded.is_empty(),
            "no seed rules found in {} — the shipped assets went missing",
            dir.display()
        );
        for l in &loaded {
            match &l.result {
                Err(e) => panic!("{}: does not parse: {e}", l.file),
                Ok(rf) => {
                    let errs = validate(rf);
                    assert!(errs.is_empty(), "{}: {errs:?}", l.file);
                }
            }
        }
    }

    #[test]
    fn automation_refuses_actions_and_searches_it_cannot_preserve() {
        for (action, method, search) in [
            ("truncate", "trash", "walk_all"),
            ("delete", "remove", "walk_all"),
            ("delete", "trash", "walk_files"),
            ("delete", "trash", "glob"),
        ] {
            let mut value: serde_json::Value = serde_json::from_str(VALID).unwrap();
            value["rules"][0]["auto"] = true.into();
            value["rules"][0]["action"] = serde_json::json!({"type": action, "method": method});
            value["rules"][0]["targets"][0]["search"] = search.into();
            assert!(
                !validate(&parse(&value.to_string()).unwrap()).is_empty(),
                "{value}"
            );
        }
    }

    #[test]
    fn unknown_conditions_and_running_guards_never_become_unconditional_rules() {
        let mut value: serde_json::Value = serde_json::from_str(VALID).unwrap();
        value["rules"][0]["targets"][0]["when"] =
            serde_json::json!({"min_age_days": 0, "max_size_bytes": 1});
        assert!(parse(&value.to_string()).is_err());
        let mut value: serde_json::Value = serde_json::from_str(VALID).unwrap();
        value["app"]["running_guard"] = serde_json::json!(true);
        assert!(parse(&value.to_string()).is_err());
    }

    #[test]
    fn a_file_search_and_directory_age_never_select_a_whole_directory() {
        let root =
            std::env::temp_dir().join(format!("burrow_autoplan_directory_{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let data = root.join("data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("recent.txt"), "keep").unwrap();
        let mut value: serde_json::Value = serde_json::from_str(VALID).unwrap();
        value["rules"][0]["auto"] = true.into();
        value["rules"][0]["targets"][0] = serde_json::json!({"path":data,"search":"file"});
        let file = root.join("rule.json");
        std::fs::write(&file, value.to_string()).unwrap();
        let (plan, skipped) = auto_clean_plan(&root, "/home/example").unwrap();
        assert!(plan.is_empty());
        assert_eq!(skipped, ["rule.json"]);
        value["rules"][0]["targets"][0] =
            serde_json::json!({"path":data,"when":{"min_age_days":0}});
        std::fs::write(&file, value.to_string()).unwrap();
        let (plan, skipped) = auto_clean_plan(&root, "/home/example").unwrap();
        assert!(plan.is_empty());
        assert_eq!(skipped, ["rule.json"]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
