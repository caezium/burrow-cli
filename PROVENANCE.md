# Provenance & clean-room log

This log records where each non-trivial capability came from and how it entered Burrow CLI.
It is the audit trail that keeps the project defensible: permissive code is *lifted with
attribution*; copyleft/closed ideas are *reimplemented clean-room* (facts and behavior, never
source). Add an entry the moment a capability is specced or code lands.

Modes: **A** = wrapped as external dependency · **B** = lifted with attribution (MIT/Apache)
· **C** = clean-room reimplementation (GPL/CC/closed — ideas only).

| Date | Capability | Source | License | Mode | Notes |
|---|---|---|---|---|---|
| 2026-06-25 | Cleaning engine (clean/uninstall/optimize/analyze/status/purge) | tw93/Mole @ `9daf936` (V1.42.0) | MIT | A | Forked under the former `burrow-engine` name; the MIT fork is now `caezium/burrow-digger`. See its `FORK_NOTICE.md`; the current Rust `caezium/burrow-engine` is separately FSL-licensed. |
| 2026-06-25 | Duplicate finder + APFS clone-dedupe (`dupes`) | fclones (pkolaczk/fclones) | MIT | A | Sidecar; stdout→stdin round-trip; `dedupe` = APFS `clonefile`. Invoked as a separate process, not linked. |
| 2026-06-25 | Rule path data (Chrome/Xcode/npm caches) | mac-cleanup-py (Apache-2.0) + kondo/npkill (MIT) | Apache-2.0 / MIT | B | Path facts lifted into `rules/*.json`, each with per-file `provenance` + `evidence`. No GPL rule data copied. |
| 2026-06-25 | Orphan confidence tiers + leftover matching | feherk/AppCleaner + Sun Knudsen (MIT), BCU confidence model (Apache); Pearcleaner (Commons-Clause) | MIT / Apache-2.0 (+ ideas) | B / C | Reimplemented in `src/orphan.rs`; Pearcleaner used as ideas only (clean-room), per the rules below. |

## Public snapshot history

This repository starts from reviewed source snapshots. Historical PR numbers and internal
commit IDs above refer to the earlier development archive; that private Git history is not
part of this publication. Upstream copyright, license text, and source identities remain
unchanged. The public `windows-legacy` branch preserves the older adapter contract separately
from the Rust-engine `main` branch.

## Clean-room rules (for Mode C work)

1. Derive **facts** (paths, conditions, heuristics) from vendor docs, man pages, or observed
   behavior — never by transcribing GPL/closed source.
2. Write a behavioral spec in our own words; implement from the spec.
3. Our code must read like our code (own naming/structure). Structural cloning is what got
   PureMac flagged — avoid it.
4. Record the source + mode here at spec time.

### Planned Mode-C items (not yet implemented)
- Orphan-scan confidence tiers — concept from Bulk Crap Uninstaller (Apache, can also lift) +
  Pearcleaner (Commons-Clause, ideas only). Implement fresh.
- Keep-newest-version heuristics (Device Support, doc caches) — concept from DevCleaner (GPL).
- Declarative rule format — structure studied from CleanerML/Tencent Lemon/privacy.sexy
  (GPL/AGPL); `burrow.rules/v1` authored fresh.
- Snapshot tree-diff — concept from gdu/gdu-diff (MIT; could also lift).
