//! Orphan / leftover scanner.
//!
//! Flags files in volatile cache/log roots that belong to NO installed app. The matching
//! logic (landscape 1.3, synthesized from feherk/AppCleaner + Sun Knudsen [MIT] and the BCU
//! confidence model [Apache]; Pearcleaner ideas reimplemented clean-room) is pure and tested;
//! only `scan` and `enumerate_installed` touch the filesystem. Read-only by design.

use crate::platform;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// How strongly a candidate file relates to an installed app. Higher = more related.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    None,
    Medium,
    Strong,
    Exact,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledApp {
    pub source: String,
    pub registry_root: Option<String>,
    pub key_path: Option<String>,
    pub display_name: Option<String>,
    pub publisher: Option<String>,
    pub install_location: Option<String>,
    pub uninstall_string: Option<String>,
    pub is_msi: bool,
    pub product_code: Option<String>,
    pub identifiers: Vec<String>,
}

impl InstalledApp {
    pub fn synthetic(source: &str, name: &str) -> Option<Self> {
        let display_name = name.trim();
        let id = normalize(display_name);
        if id.len() < 3 {
            return None;
        }
        let mut app = Self {
            source: source.to_string(),
            registry_root: None,
            key_path: None,
            display_name: Some(display_name.to_string()),
            publisher: None,
            install_location: None,
            uninstall_string: None,
            is_msi: false,
            product_code: None,
            identifiers: Vec::new(),
        };
        app.rebuild_identifiers();
        Some(app)
    }

    fn from_registry_entry(
        source: &str,
        registry_root: &str,
        key_path: &str,
        values: &BTreeMap<String, RegistryValue>,
    ) -> Option<Self> {
        if registry_dword_truthy(values.get("SystemComponent").map(|v| v.value.as_str())) {
            return None;
        }

        let display_name = nonempty_registry_value(values, "DisplayName")?;
        let mut app = Self {
            source: source.to_string(),
            registry_root: Some(registry_root.to_string()),
            key_path: Some(key_path.to_string()),
            display_name: Some(display_name),
            publisher: nonempty_registry_value(values, "Publisher"),
            install_location: nonempty_registry_value(values, "InstallLocation"),
            uninstall_string: nonempty_registry_value(values, "UninstallString"),
            is_msi: registry_dword_truthy(values.get("WindowsInstaller").map(|v| v.value.as_str())),
            product_code: product_code_from_key(key_path),
            identifiers: Vec::new(),
        };
        app.rebuild_identifiers();
        Some(app)
    }

    fn rebuild_identifiers(&mut self) {
        let mut ids = Vec::new();
        for value in [
            self.display_name.as_deref(),
            self.publisher.as_deref(),
            self.product_code.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            let norm = normalize(value);
            if norm.len() >= 3 {
                ids.push(norm);
            }
        }
        if let Some(location) = &self.install_location {
            if let Some(name) = path_leaf(location) {
                let norm = normalize(&name);
                if norm.len() >= 3 {
                    ids.push(norm);
                }
            }
        }
        ids.sort();
        ids.dedup();
        self.identifiers = ids;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegistryValue {
    value: String,
}

/// Lowercase + ASCII-alphanumeric only (drops dots/dashes/spaces).
pub fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn nonempty_registry_value(values: &BTreeMap<String, RegistryValue>, name: &str) -> Option<String> {
    values.get(name).and_then(|v| {
        let trimmed = v.value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn registry_dword_truthy(value: Option<&str>) -> bool {
    let Some(value) = value else {
        return false;
    };
    let trimmed = value.trim().to_ascii_lowercase();
    trimmed == "1" || trimmed == "0x1" || trimmed == "0x00000001"
}

fn path_leaf(path: &str) -> Option<String> {
    path.trim_matches('"')
        .trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .find(|p| !p.trim().is_empty())
        .map(|p| p.trim().to_string())
}

fn product_code_from_key(key_path: &str) -> Option<String> {
    let leaf = path_leaf(key_path)?;
    if is_guid_like(&leaf) {
        Some(leaf)
    } else {
        None
    }
}

fn is_guid_like(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 38 || bytes.first() != Some(&b'{') || bytes.last() != Some(&b'}') {
        return false;
    }
    for (i, b) in bytes[1..37].iter().enumerate() {
        let pos = i + 1;
        if matches!(pos, 9 | 14 | 19 | 24) {
            if *b != b'-' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

/// Normalize a candidate filename, dropping volatile tokens first (duplicate counters,
/// long digit runs like dates/versions, and hex runs like UUIDs/hashes) so they don't
/// dilute the match — the app-eraser insight.
pub fn clean_name(raw: &str) -> String {
    let no_counters = remove_paren_counters(raw);
    let lower = no_counters.to_lowercase();
    let mut out = String::new();
    for tok in lower.split(|c: char| !c.is_ascii_alphanumeric()) {
        if tok.is_empty() {
            continue;
        }
        let all_digits = tok.bytes().all(|b| b.is_ascii_digit());
        let all_hex = tok.bytes().all(|b| b.is_ascii_hexdigit());
        if all_digits {
            continue; // counters / dates / versions are noise for identity matching
        }
        if all_hex && tok.len() >= 12 {
            continue; // UUID chunk / content hash
        }
        out.push_str(tok);
    }
    out
}

fn remove_paren_counters(s: &str) -> String {
    // drop "(<digits>)" runs, e.g. "Slack (2)" -> "Slack "
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 && j < bytes.len() && bytes[j] == b')' {
                i = j + 1; // skip "(123)"
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn longest_common_substring_len(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let mut prev = vec![0usize; b.len() + 1];
    let mut best = 0;
    for i in 1..=a.len() {
        let mut curr = vec![0usize; b.len() + 1];
        for j in 1..=b.len() {
            if a[i - 1] == b[j - 1] {
                curr[j] = prev[j - 1] + 1;
                best = best.max(curr[j]);
            }
        }
        prev = curr;
    }
    best
}

/// Fraction of the candidate explained by the best contiguous overlap with `id`.
pub fn coverage_ratio(cand: &str, id: &str) -> f64 {
    let denom = cand.chars().count();
    if denom == 0 {
        return 0.0;
    }
    longest_common_substring_len(cand, id) as f64 / denom as f64
}

/// Best relatedness of a candidate (raw filename) to any installed identifier
/// (pre-normalized via `normalize`).
pub fn relatedness(candidate_raw: &str, installed: &[String]) -> Confidence {
    let cand = clean_name(candidate_raw);
    if cand.len() < 3 {
        return Confidence::None;
    }
    let mut best = Confidence::None;
    for id in installed {
        if id.len() < 3 {
            continue;
        }
        let c = if cand == *id {
            Confidence::Exact
        } else if id.len() >= 5 && cand.contains(id.as_str()) {
            Confidence::Strong
        } else if coverage_ratio(&cand, id) > 0.4 {
            Confidence::Medium
        } else {
            Confidence::None
        };
        best = best.max(c);
        if best == Confidence::Exact {
            break;
        }
    }
    best
}

/// Apple/system files that must never be flagged as orphans.
pub fn is_safelisted(name: &str) -> bool {
    let l = name.to_lowercase();
    l.starts_with("com.apple.")
        || l.starts_with("is.workflow.")
        || l == ".ds_store"
        || l.ends_with(".globalpreferences.plist")
}

/// Whether the name looks like an app-specific artifact (bundle-id-ish: >=3 dot components,
/// after stripping group/systemgroup prefixes). Prevents flagging arbitrary user files.
pub fn looks_like_app_artifact(name: &str) -> bool {
    let base = name
        .strip_prefix("group.")
        .or_else(|| name.strip_prefix("systemgroup."))
        .unwrap_or(name);
    base.split('.').filter(|p| !p.is_empty()).count() >= 3
}

/// A candidate is an orphan iff it looks like an app artifact, is not safelisted, and relates
/// to no installed app.
pub fn is_orphan(candidate_raw: &str, installed: &[String]) -> bool {
    if is_safelisted(candidate_raw) || !looks_like_app_artifact(candidate_raw) {
        return false;
    }
    relatedness(candidate_raw, installed) == Confidence::None
}

#[derive(Debug, Serialize)]
pub struct OrphanHit {
    pub name: String,
    pub path: String,
    pub confidence: &'static str,
    pub evidence: Vec<&'static str>,
    pub default_selected: bool,
}

/// Scan a root for orphan candidates given the installed-app inventory.
pub fn scan(root: &Path, installed: &[InstalledApp]) -> Vec<OrphanHit> {
    if platform::is_windows() {
        return scan_windows(root, installed);
    }
    scan_legacy(root, &installed_identifiers(installed))
}

fn installed_identifiers(installed: &[InstalledApp]) -> Vec<String> {
    let mut ids: Vec<String> = installed
        .iter()
        .flat_map(|app| app.identifiers.iter().cloned())
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// Scan a single directory (non-recursive) with the original bundle-id-style matcher.
fn scan_legacy(root: &Path, installed: &[String]) -> Vec<OrphanHit> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if is_orphan(&name, installed) {
            hits.push(OrphanHit {
                name,
                path: e.path().to_string_lossy().into_owned(),
                confidence: "medium",
                evidence: vec!["app-artifact-shaped", "not-matched-to-installed-inventory"],
                default_selected: false,
            });
        }
    }
    hits.sort_by(|a, b| a.name.cmp(&b.name));
    hits
}

fn scan_windows(root: &Path, installed: &[InstalledApp]) -> Vec<OrphanHit> {
    let matchers = build_matchers(installed);
    let max_depth = windows_scan_depth(root);
    let mut hits = Vec::new();
    scan_windows_at_depth(root, &matchers, 1, max_depth, &mut hits);
    hits.sort_by(|a, b| a.path.cmp(&b.path));
    hits.dedup_by(|a, b| a.path == b.path);
    hits
}

fn scan_windows_at_depth(
    root: &Path,
    matchers: &[AppMatcher],
    depth: usize,
    max_depth: usize,
    hits: &mut Vec<OrphanHit>,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(hit) = windows_orphan_hit_for_path(&path, matchers) {
            hits.push(hit);
        }
        if depth < max_depth && path.is_dir() {
            scan_windows_at_depth(&path, matchers, depth + 1, max_depth, hits);
        }
    }
}

fn windows_scan_depth(root: &Path) -> usize {
    for key in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(value) = std::env::var(key) {
            if path_strings_equal(root, &value) {
                return 1;
            }
        }
    }
    2
}

/// Precomputed, normalized matching keys for one installed app. Built once per scan so
/// the per-entry hot loop never re-normalizes display names or re-expands install paths
/// (was O(entries × apps) `normalize`/env-var work; now O(apps) up front).
struct AppMatcher {
    display_id: Option<String>,
    publisher_id: Option<String>,
    /// Env-expanded, non-empty install location (see `expand_windows_env_vars`).
    install_location: Option<String>,
}

fn build_matchers(installed: &[InstalledApp]) -> Vec<AppMatcher> {
    installed
        .iter()
        .map(|app| AppMatcher {
            display_id: app
                .display_name
                .as_deref()
                .map(normalize)
                .filter(|s| s.len() >= 3),
            publisher_id: app
                .publisher
                .as_deref()
                .map(normalize)
                .filter(|s| s.len() >= 3),
            install_location: app
                .install_location
                .as_deref()
                .map(expand_windows_env_vars)
                .filter(|s| !s.trim().is_empty()),
        })
        .collect()
}

/// Expand `%VAR%` tokens using the process environment. Registry `InstallLocation` values
/// are stored raw (`REG_EXPAND_SZ`, e.g. `%ProgramFiles%\Foo`), so they must be expanded
/// before comparing against real filesystem paths — otherwise the "is this path inside the
/// install dir" signal silently never fires. Unknown/malformed tokens are left intact.
fn expand_windows_env_vars(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                if name.is_empty() {
                    out.push('%'); // "%%" -> literal %
                } else if let Ok(val) = std::env::var(name) {
                    out.push_str(&val);
                } else {
                    out.push('%');
                    out.push_str(name);
                    out.push('%');
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                out.push_str(after);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct MatchOutcome {
    matched: bool,
    evidence: Vec<&'static str>,
}

fn windows_orphan_hit_for_path(path: &Path, matchers: &[AppMatcher]) -> Option<OrphanHit> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    if is_safelisted(&name) || is_windows_safelisted(&name) {
        return None;
    }
    if candidate_matches(path, &name, matchers).matched {
        return None;
    }
    if !looks_like_windows_app_artifact(path, &name) {
        return None;
    }

    let mut evidence = Vec::new();
    if is_known_cache_log_path(path) {
        evidence.push("known-cache-log-path");
    }
    evidence.push("unmatched-app-artifact-shaped-path");
    let confidence = if evidence.contains(&"known-cache-log-path") {
        "medium"
    } else {
        "low"
    };
    Some(OrphanHit {
        name,
        path: path.to_string_lossy().into_owned(),
        confidence,
        evidence,
        default_selected: false,
    })
}

fn candidate_matches(path: &Path, name: &str, matchers: &[AppMatcher]) -> MatchOutcome {
    let path_tokens = normalized_path_tokens(path);
    let mut outcome = MatchOutcome::default();
    for m in matchers {
        if let Some(location) = &m.install_location {
            if path_is_same_or_inside(path, location) || path_leaf_matches(path, location) {
                push_unique(&mut outcome.evidence, "install-location-match");
                outcome.matched = true;
            }
        }

        if let Some(display_id) = &m.display_id {
            let name_match =
                relatedness(name, std::slice::from_ref(display_id)) >= Confidence::Strong;
            let path_match = path_tokens
                .iter()
                .any(|token| token == display_id || token.contains(display_id));
            if name_match || path_match {
                push_unique(&mut outcome.evidence, "display-name-match");
                outcome.matched = true;
            }
        }

        if let (Some(display_id), Some(publisher_id)) = (&m.display_id, &m.publisher_id) {
            let has_publisher = path_tokens
                .iter()
                .any(|token| token == publisher_id || token.contains(publisher_id));
            let has_product = path_tokens
                .iter()
                .any(|token| token == display_id || token.contains(display_id));
            if has_publisher && has_product {
                push_unique(&mut outcome.evidence, "publisher-match");
                push_unique(&mut outcome.evidence, "display-name-match");
                outcome.matched = true;
            }
        }
    }
    outcome
}

#[cfg(test)]
fn windows_candidate_matches_installed(
    path: &Path,
    name: &str,
    installed: &[InstalledApp],
) -> MatchOutcome {
    candidate_matches(path, name, &build_matchers(installed))
}

fn push_unique(values: &mut Vec<&'static str>, value: &'static str) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn normalized_path_tokens(path: &Path) -> Vec<String> {
    path.to_string_lossy()
        .split(['\\', '/'])
        .map(clean_name)
        .filter(|token| token.len() >= 3)
        .collect()
}

fn normalized_path_string(path: &str) -> String {
    path.trim_matches('"')
        .trim_end_matches(['\\', '/'])
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn path_strings_equal(path: &Path, other: &str) -> bool {
    normalized_path_string(&path.to_string_lossy()) == normalized_path_string(other)
}

fn path_is_same_or_inside(path: &Path, parent: &str) -> bool {
    let path = normalized_path_string(&path.to_string_lossy());
    let parent = normalized_path_string(parent);
    path == parent || path.starts_with(&format!("{parent}/"))
}

fn path_leaf_matches(path: &Path, other: &str) -> bool {
    let Some(other_leaf) = path_leaf(other) else {
        return false;
    };
    let Some(path_leaf) = path.file_name().map(|p| p.to_string_lossy()) else {
        return false;
    };
    normalize(&path_leaf) == normalize(&other_leaf)
}

fn looks_like_windows_app_artifact(path: &Path, name: &str) -> bool {
    if looks_like_app_artifact(name) {
        return true;
    }
    let norm = normalize(name);
    if norm.len() < 3 || is_windows_safelisted(name) {
        return false;
    }
    if has_common_user_file_extension(name) {
        return false;
    }
    // A plain (non-bundle-id-shaped) directory only counts as an artifact when its own name
    // signals a leftover (cache/log/temp). Flagging *every* unmatched directory floods the
    // low-confidence bucket with legitimate, unregistered app data on real systems.
    path.is_dir() && name_suggests_leftover(name)
}

fn has_common_user_file_extension(name: &str) -> bool {
    let Some(ext) = Path::new(name).extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "txt"
            | "md"
            | "pdf"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "doc"
            | "docx"
            | "xls"
            | "xlsx"
            | "zip"
            | "exe"
            | "dll"
            | "lnk"
    )
}

fn is_windows_safelisted(name: &str) -> bool {
    let n = normalize(name);
    matches!(
        n.as_str(),
        "microsoft"
            | "windows"
            | "windowsapps"
            | "packages"
            | "temp"
            | "tmp"
            | "commonfiles"
            | "programfiles"
            | "programfilesx86"
            | "desktop"
            | "documents"
            | "downloads"
            | "startmenu"
            | "startup"
            | "users"
            | "public"
    ) || n.starts_with("microsoftwindows")
}

fn is_known_cache_log_path(path: &Path) -> bool {
    normalized_path_tokens(path)
        .iter()
        .any(|token| is_leftover_token(token))
}

/// Whether a directory's *own* name (not its ancestors) marks it as leftover-shaped. Used
/// to gate the plain-directory artifact heuristic; unlike `is_known_cache_log_path` it does
/// not consider ancestor folders, so e.g. a plain vendor dir under `%TEMP%` isn't swept in.
fn name_suggests_leftover(name: &str) -> bool {
    is_leftover_token(&normalize(name))
}

fn is_leftover_token(token: &str) -> bool {
    matches!(
        token,
        "cache" | "caches" | "log" | "logs" | "crashdump" | "crashdumps" | "temp"
    ) || token.ends_with("cache")
        || token.ends_with("logs")
}

/// Enumerate installed apps from the current platform inventory source.
pub fn enumerate_installed_apps() -> Vec<InstalledApp> {
    if platform::is_windows() {
        return enumerate_windows_installed_apps();
    }
    let mut apps = Vec::new();
    let mut roots = vec![PathBuf::from("/Applications")];
    roots.push(platform::home_dir().join("Applications"));
    for root in roots {
        if let Ok(entries) = std::fs::read_dir(&root) {
            for e in entries.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if let Some(app) = n.strip_suffix(".app") {
                    let norm = normalize(app);
                    if norm.len() >= 3 {
                        if let Some(app) = InstalledApp::synthetic("applications_dir", app) {
                            apps.push(app);
                        }
                    }
                }
            }
        }
    }
    apps.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    apps.dedup_by(|a, b| a.display_name == b.display_name);
    apps
}

pub fn installed_from_cli_csv(csv: &str) -> Vec<InstalledApp> {
    let mut apps: Vec<InstalledApp> = csv
        .split(',')
        .filter_map(|s| InstalledApp::synthetic("cli", s))
        .collect();
    apps.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    apps.dedup_by(|a, b| a.identifiers == b.identifiers);
    apps
}

pub fn inventory_source_counts(installed: &[InstalledApp]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for app in installed {
        *counts.entry(app.source.clone()).or_insert(0) += 1;
    }
    counts
}

pub fn default_scan_roots() -> Vec<PathBuf> {
    if !platform::is_windows() {
        return Vec::new();
    }
    let mut roots = Vec::new();
    for key in [
        "APPDATA",
        "LOCALAPPDATA",
        "PROGRAMDATA",
        "ProgramFiles",
        "ProgramFiles(x86)",
    ] {
        if let Ok(v) = std::env::var(key) {
            roots.push(PathBuf::from(v));
        }
    }
    roots.sort();
    roots.dedup();
    roots
}

const WINDOWS_REGISTRY_ROOTS: [(&str, &str); 3] = [
    (
        "registry_hkcu_uninstall",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall",
    ),
    (
        "registry_hklm_uninstall",
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall",
    ),
    (
        "registry_wow6432_uninstall",
        r"HKLM\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    ),
];

fn enumerate_windows_installed_apps() -> Vec<InstalledApp> {
    let mut apps = Vec::new();
    for (source, root) in WINDOWS_REGISTRY_ROOTS {
        if let Ok(out) = std::process::Command::new("reg")
            .args(["query", root, "/s"])
            .output()
        {
            if out.status.success() {
                apps.extend(parse_windows_registry_uninstall(
                    &String::from_utf8_lossy(&out.stdout),
                    source,
                    root,
                ));
            }
        }
    }
    apps.sort_by(|a, b| {
        a.display_name
            .cmp(&b.display_name)
            .then(a.source.cmp(&b.source))
            .then(a.key_path.cmp(&b.key_path))
    });
    apps.dedup_by(|a, b| {
        a.display_name == b.display_name
            && a.publisher == b.publisher
            && a.install_location == b.install_location
    });
    apps
}

pub fn parse_windows_registry_uninstall(
    output: &str,
    source: &str,
    registry_root: &str,
) -> Vec<InstalledApp> {
    let mut apps = Vec::new();
    let mut current_key: Option<String> = None;
    let mut values: BTreeMap<String, RegistryValue> = BTreeMap::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("HKEY_") {
            flush_registry_entry(
                &mut apps,
                source,
                registry_root,
                current_key.take(),
                &mut values,
            );
            current_key = Some(trimmed.to_string());
            continue;
        }
        if current_key.is_some() {
            if let Some((name, value)) = parse_registry_value_line(trimmed) {
                values.insert(name, value);
            }
        }
    }
    flush_registry_entry(
        &mut apps,
        source,
        registry_root,
        current_key.take(),
        &mut values,
    );
    apps
}

fn flush_registry_entry(
    apps: &mut Vec<InstalledApp>,
    source: &str,
    registry_root: &str,
    key: Option<String>,
    values: &mut BTreeMap<String, RegistryValue>,
) {
    if let Some(key) = key {
        if let Some(app) = InstalledApp::from_registry_entry(source, registry_root, &key, values) {
            apps.push(app);
        }
    }
    values.clear();
}

fn parse_registry_value_line(line: &str) -> Option<(String, RegistryValue)> {
    let mut parts = line.split_whitespace();
    let name = parts.next()?;
    let kind = parts.next()?;
    if !kind.starts_with("REG_") {
        return None;
    }
    let kind_pos = line.find(kind)?;
    let value_start = kind_pos + kind.len();
    let value = line[value_start..].trim();
    Some((
        name.to_string(),
        RegistryValue {
            value: value.to_string(),
        },
    ))
}

#[cfg(test)]
pub fn parse_registry_display_names(output: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for (source, root) in WINDOWS_REGISTRY_ROOTS {
        ids.extend(
            parse_windows_registry_uninstall(output, source, root)
                .into_iter()
                .filter_map(|app| app.display_name.map(|name| normalize(&name)))
                .filter(|id| id.len() >= 3),
        );
    }
    ids.sort();
    ids.dedup();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| normalize(s)).collect()
    }

    fn win_app(
        display_name: &str,
        publisher: Option<&str>,
        install_location: Option<&str>,
    ) -> InstalledApp {
        let mut app = InstalledApp::synthetic("registry_hkcu_uninstall", display_name).unwrap();
        app.publisher = publisher.map(str::to_string);
        app.install_location = install_location.map(str::to_string);
        app.rebuild_identifiers();
        app
    }

    #[test]
    fn normalize_strips_punctuation() {
        assert_eq!(normalize("Com.Google.Chrome!"), "comgooglechrome");
    }

    #[test]
    fn clean_name_drops_counters_dates_uuids() {
        assert_eq!(clean_name("Slack (2)"), "slack");
        assert_eq!(clean_name("com.foo.bar-2024-01-01"), "comfoobar");
        // a 32-hex uuid chunk is dropped
        assert_eq!(
            clean_name("cache-0123456789abcdef0123456789abcdef"),
            "cache"
        );
    }

    #[test]
    fn exact_and_strong_matches_are_related() {
        let installed = ids(&["com.google.Chrome"]);
        // helper file for an installed app -> Strong (contains the bundle id)
        assert_eq!(
            relatedness("com.google.Chrome.savedState", &installed),
            Confidence::Strong
        );
        // exact
        assert_eq!(
            relatedness("com.google.Chrome", &installed),
            Confidence::Exact
        );
    }

    #[test]
    fn unrelated_bundle_is_orphan() {
        let installed = ids(&["com.google.Chrome"]);
        assert!(is_orphan("com.deadvendor.oldapp.savedState", &installed));
    }

    #[test]
    fn apple_files_never_orphan() {
        let installed = ids(&["com.google.Chrome"]);
        assert!(!is_orphan("com.apple.Safari.savedState", &installed));
        assert!(!is_orphan(".DS_Store", &installed));
    }

    #[test]
    fn non_bundle_files_never_orphan() {
        let installed = ids(&["com.google.Chrome"]);
        // not bundle-id-shaped -> never flagged, even if unrelated
        assert!(!is_orphan("my-notes.txt", &installed));
        assert!(!is_orphan("Screenshot.png", &installed));
    }

    #[test]
    fn installed_app_files_not_orphan() {
        let installed = ids(&["com.google.Chrome", "com.tinyspeck.slackmacgap"]);
        assert!(!is_orphan("com.tinyspeck.slackmacgap.helper", &installed));
    }

    #[test]
    fn coverage_ratio_partial() {
        // "comslack" shares "slack" (5) with "slack" -> 5/8
        assert!((coverage_ratio("comslack", "slack") - 0.625).abs() < 1e-9);
    }

    #[test]
    fn parses_windows_registry_display_names() {
        let out = r#"
HKEY_LOCAL_MACHINE\Software\Microsoft\Windows\CurrentVersion\Uninstall\Foo
    DisplayName    REG_SZ    Foo App
    Publisher      REG_SZ    Example
HKEY_LOCAL_MACHINE\Software\Microsoft\Windows\CurrentVersion\Uninstall\Bar
    DisplayName    REG_SZ    Bar Tool
"#;
        let ids = parse_registry_display_names(out);
        assert!(ids.contains(&normalize("Foo App")));
        assert!(ids.contains(&normalize("Bar Tool")));
    }

    #[test]
    fn parses_windows_registry_fixtures_into_installed_apps() {
        let hkcu = parse_windows_registry_uninstall(
            include_str!("../testdata/windows/registry/hkcu_uninstall.txt"),
            "registry_hkcu_uninstall",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall",
        );
        assert_eq!(hkcu.len(), 1, "hidden and nameless entries must be skipped");
        assert_eq!(hkcu[0].source, "registry_hkcu_uninstall");
        assert_eq!(hkcu[0].display_name.as_deref(), Some("Example User App"));
        assert_eq!(hkcu[0].publisher.as_deref(), Some("Example Software LLC"));
        assert_eq!(
            hkcu[0].install_location.as_deref(),
            Some(r"%LOCALAPPDATA%\Programs\Example User App")
        );

        let hklm = parse_windows_registry_uninstall(
            include_str!("../testdata/windows/registry/hklm_uninstall.txt"),
            "registry_hklm_uninstall",
            r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall",
        );
        assert_eq!(hklm.len(), 2);
        let msi = hklm
            .iter()
            .find(|app| app.display_name.as_deref() == Some("MSI Fixture App"))
            .unwrap();
        assert!(msi.is_msi);
        assert_eq!(
            msi.product_code.as_deref(),
            Some("{12345678-1234-ABCD-9876-1234567890AB}")
        );
        assert_eq!(
            msi.install_location.as_deref(),
            Some(r"%ProgramFiles%\MSI Fixture App")
        );

        let wow = parse_windows_registry_uninstall(
            include_str!("../testdata/windows/registry/wow6432_uninstall.txt"),
            "registry_wow6432_uninstall",
            r"HKLM\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        );
        assert_eq!(wow.len(), 1);
        assert_eq!(wow[0].source, "registry_wow6432_uninstall");
        assert_eq!(wow[0].display_name.as_deref(), Some("Legacy 32 Tool"));
    }

    #[test]
    fn windows_display_name_match_suppresses_appdata_leftover() {
        let app = win_app("Example User App", None, None);
        let path = Path::new("C:/Users/Alice/AppData/Roaming/Example User App");
        let outcome = windows_candidate_matches_installed(path, "Example User App", &[app]);
        assert!(outcome.matched);
        assert!(outcome.evidence.contains(&"display-name-match"));
    }

    #[test]
    fn windows_install_location_match_suppresses_program_files_path() {
        let app = win_app(
            "Machine Tool Pro",
            Some("Machine Tools Incorporated"),
            Some(r"C:\Program Files\Machine Tool Pro"),
        );
        let path = Path::new("C:/Program Files/Machine Tool Pro/cache");
        let outcome = windows_candidate_matches_installed(path, "cache", &[app]);
        assert!(outcome.matched);
        assert!(outcome.evidence.contains(&"install-location-match"));
    }

    #[test]
    fn windows_publisher_only_match_is_insufficient() {
        let app = win_app("Machine Tool Pro", Some("Machine Tools Incorporated"), None);
        let path = Path::new("C:/Users/Alice/AppData/Roaming/Machine Tools Incorporated");
        let outcome =
            windows_candidate_matches_installed(path, "Machine Tools Incorporated", &[app]);
        assert!(!outcome.matched, "publisher alone must not suppress a hit");
    }

    #[test]
    fn windows_unmatched_appdata_path_has_evidence_and_is_not_selected() {
        let dir = std::env::temp_dir()
            .join(format!("burrow_orphan_win_{}", std::process::id()))
            .join("Ghost Cache");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let hit = windows_orphan_hit_for_path(&dir, &[]).unwrap();
        assert_eq!(hit.confidence, "medium");
        assert!(!hit.default_selected);
        assert!(hit.evidence.contains(&"known-cache-log-path"));
        assert!(hit.evidence.contains(&"unmatched-app-artifact-shaped-path"));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn windows_common_system_folders_are_safelisted() {
        for name in ["Microsoft", "WindowsApps", "Common Files", "Start Menu"] {
            assert!(is_windows_safelisted(name), "{name} should be safelisted");
            let path = Path::new("C:/ProgramData").join(name);
            assert!(windows_orphan_hit_for_path(&path, &[]).is_none());
        }
    }

    #[test]
    fn windows_install_location_env_var_is_expanded_for_matching() {
        // Registry stores InstallLocation raw (REG_EXPAND_SZ). Matching must expand %VARS%
        // before comparing to real paths, or a subfolder inside a genuinely-installed app
        // whose folder name differs from its display name gets false-flagged. Display name
        // ("Contoso Suite") is deliberately unrelated to the install folder ("CTS") so ONLY
        // the expanded install-location signal can suppress the hit.
        std::env::set_var("BURROW_TEST_INSTALL_ROOT", r"C:\Program Files");
        let app = win_app(
            "Contoso Suite",
            None,
            Some(r"%BURROW_TEST_INSTALL_ROOT%\CTS"),
        );
        let path = Path::new("C:/Program Files/CTS/telemetry");
        let outcome = windows_candidate_matches_installed(path, "telemetry", &[app]);
        std::env::remove_var("BURROW_TEST_INSTALL_ROOT");
        assert!(
            outcome.matched,
            "env-var install location must expand and suppress the subfolder"
        );
        assert!(outcome.evidence.contains(&"install-location-match"));
    }

    #[test]
    fn windows_plain_unmatched_dir_without_leftover_signal_is_not_flagged() {
        // Noise control: an unmatched directory whose own name carries no cache/log/temp
        // signal (e.g. a live app's data folder that just isn't in the registry) must not be
        // flagged — regardless of the ancestor folders it sits under (e.g. under %TEMP%).
        let dir = std::env::temp_dir()
            .join(format!("burrow_orphan_plain_{}", std::process::id()))
            .join("SomeVendor");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        assert!(
            windows_orphan_hit_for_path(&dir, &[]).is_none(),
            "plain unmatched dir without a leftover signal must not be flagged"
        );
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
