//! `burrow.rules/v1` — declarative, agent-readable per-app cleaning rules.
//!
//! Design (landscape doc 1.2): per-app rules keyed by bundle id, a CLOSED action enum (no
//! arbitrary shell — every rule is statically auditable and dry-runnable), risk tiers that
//! drive default selection, trash-vs-remove semantics, and a required `provenance` block with
//! evidence citations (the agent-native differentiator). The format is JSON for agent-nativeness.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Deserialize, Serialize)]
pub struct RuleFile {
    pub schema: String,
    pub app: AppSpec,
    #[serde(default)]
    pub rules: Vec<Rule>,
    pub provenance: Provenance,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AppSpec {
    pub bundle_ids: Vec<String>,
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Rule {
    pub id: String,
    pub category: String,
    pub risk: Risk,
    #[serde(default)]
    pub recommend: bool,
    #[serde(default)]
    pub explain: Option<String>,
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

impl Risk {
    pub fn as_str(&self) -> &'static str {
        match self {
            Risk::Safe => "safe",
            Risk::Caution => "caution",
            Risk::Risky => "risky",
        }
    }
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
pub struct Target {
    pub path: String,
    #[serde(default)]
    pub search: Search,
}

#[derive(Debug, Deserialize, Serialize)]
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

impl Method {
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Trash => "trash",
            Method::Remove => "remove",
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
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
pub fn load_dir(dir: &Path) -> Result<Vec<Loaded>, String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut paths: Vec<_> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
        .collect();
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
