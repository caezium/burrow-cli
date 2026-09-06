# Provenance & clean-room log

This log records where each non-trivial capability came from and how it entered Burrow CLI.
It is the audit trail that keeps the project defensible: permissive code is *lifted with
attribution*; copyleft/closed ideas are *reimplemented clean-room* (facts and behavior, never
source). Add an entry the moment a capability is specced or code lands.

Modes: **A** = wrapped as external dependency · **B** = lifted with attribution (MIT/Apache)
· **C** = clean-room reimplementation (GPL/CC/closed — ideas only).

> **Reading the file paths below.** Rows are a dated record of what entered the codebase and
> when — they are left exactly as written, because an audit trail that gets rewritten every
> refactor stops being evidence. But #18 moved eight capabilities to `burrow-engine`, so some
> paths no longer resolve in this repo. (The rows and the migration record below cite the
> pre-squash commits `b5768ad` and `3633c19`; #18 was squash-merged, so on `main` both are
> `8cff30f`.) Translation, so a licence audit can still find the code:
>
> | Row says | Where it is now |
> |---|---|
> | `src/net.rs` (per-app network) | `burrow-engine`, `src/net/mod.rs` — deleted here. The migration initially dropped the Windows half; the current engine has since restored IP Helper/`netstat` connection attribution and its target-scoped `windows-sys` dependency |
> | `src/orphan.rs` (orphan confidence tiers) | `burrow-engine`, `src/orphan/mod.rs` — deleted here. macOS scanner moved; the `reg query` uninstall-registry inventory was **dropped, not moved** |
> | `src/evict.rs` (cloud-file dehydration) | `burrow-engine`, `src/evict/mod.rs` — deleted here. The macOS `brctl` path moved; the provider-aware OneDrive/Cloud-Files half was **dropped, not moved** |
> | `src/dupes.rs` (fclones driver) | `burrow-engine`, `src/dupes/mod.rs` — deleted here. fclones is still a sidecar process, invoked by the engine rather than by this crate; the czkawka fallback was **dropped, not moved** (the czkawka driver itself survives here as `win-dupes`) |
> | `src/photos.rs` (dHash similar photos) | `burrow-engine`, `src/photos/` — deleted here, and the `image` crate went with it (no longer a `burrow-cli` dependency; the engine links and attributes it) |
> | `src/rules.rs` (rule format) | **split.** The engine serves the `rules` command (`src/rules/mod.rs`); the conductor keeps its copy, because `trigger --rules` has no engine equivalent |
> | `src/macho.rs` (Mach-O fat slicing) | **split.** The read-only `slim-check` analysis is the engine's (`src/macho/mod.rs`); the `slim` **write** path — thinning + re-sign — is retained here and is still this repo's only implementation |
> | `src/sentinel.rs` (trashed-app watch) | **split.** The one-shot `sentinel` scan is the engine's (`src/sentinel/mod.rs`); the `--watch` NDJSON daemon the launchd templates run is retained here, because the engine has no watch mode and refuses the flag |
> | `src/treediff.rs`, `src/recycle.rs`, `src/bcu.rs`, `src/czkawka.rs` | unchanged, still here |
>
> Nothing about the *sources*, *licences* or *modes* recorded below changed — only which binary
> the code compiles into, and in three cases only *half* of it. In particular no licence changed:
> `caezium/burrow-engine` carries the same FSL-1.1-ALv2 and the same copyright holder as this
> repo. See [the migration record](#the-2026-08-08-engine-migration) below for what that means
> for an audit, and for what it does **not** establish.

| Date | Capability | Source | License | Mode | Notes |
|---|---|---|---|---|---|
| 2026-06-25 | Cleaning engine (clean/uninstall/optimize/analyze/status/purge) | tw93/Mole @ `9daf936` (V1.42.0) | MIT | A | Forked at the last MIT commit before upstream's GPL relicense. That fork is now `caezium/burrow-digger`; its `FORK_NOTICE.md` is the record (still self-titled "burrow-engine", which is what the fork was called before `32ec0f0` freed the name for the Rust core). It is **not** the Rust `caezium/burrow-engine`, which is a separate repo — see the migration record below. |
| 2026-06-25 | Duplicate finder + APFS clone-dedupe (`dupes`) | fclones (pkolaczk/fclones) | MIT | A | Sidecar; stdout→stdin round-trip; `dedupe` = APFS `clonefile`. Invoked as a separate process, not linked. |
| 2026-06-25 | Rule path data (Chrome/Xcode/npm caches) | mac-cleanup-py (Apache-2.0) + kondo/npkill (MIT) | Apache-2.0 / MIT | B | Path facts lifted into `rules/*.json`, each with per-file `provenance` + `evidence`. No GPL rule data copied. |
| 2026-06-25 | Orphan confidence tiers + leftover matching | feherk/AppCleaner + Sun Knudsen (MIT), BCU confidence model (Apache); Pearcleaner (Commons-Clause) | MIT / Apache-2.0 (+ ideas) | B / C | Reimplemented in `src/orphan.rs`; Pearcleaner used as ideas only (clean-room), per the rules below. |
| 2026-07-11 | Declarative rule format (`burrow.rules/v1`) | structure studied from CleanerML / Tencent Lemon / privacy.sexy | GPL/AGPL (ideas only) | C | Format authored fresh in `src/rules.rs`; path *data* is the lifted kondo/mac-cleanup-py entry above. Entry recorded retroactively. |
| 2026-07-11 | Snapshot tree-diff (`diff`) | concept from gdu/gdu-diff | MIT (ideas only) | C | Clean-room in `src/treediff.rs`; nothing lifted. Recorded retroactively. |
| 2026-07-11 | Similar photos (`photos`) | dHash — classic public algorithm | — | C | Clean-room in `src/photos.rs`; PNG/JPEG decoding via the linked `image` crate (MIT/Apache-2.0). Recorded retroactively. |
| 2026-07-11 | App slim (`slim`/`slim-check`) | Pearcleaner (behavior reference) | Commons-Clause (ideas only) | C | Clean-room Mach-O fat-slice thinning + re-sign in `src/macho.rs`. Recorded retroactively. |
| 2026-07-11 | Windows uninstall adapter (`win-uninstall`) | Bulk Crap Uninstaller (BCU-console) | Apache-2.0 | A | Sidecar process; command-building only, in `src/bcu.rs`. Not linked. Recorded retroactively. |
| 2026-07-11 | Windows dupes/similar-photos adapter (`win-dupes`) | czkawka_cli | MIT | A | Sidecar process, `src/czkawka.rs`; GPL Krokiet/Cedinia frontends not used. Recorded retroactively. |
| 2026-07-11 | Safe delete (`trash`) | `trash` crate | MIT | A | Linked Rust dependency, used from `src/recycle.rs` (Trash on macOS, Recycle Bin on Windows). Recorded retroactively. |
| 2026-07-11 | Low-disk auto-clean trigger (`trigger`) | Windows Storage Sense (concept) | closed (ideas only) | C | Threshold-trigger concept only; implemented fresh in `src/metrics.rs`. Recorded retroactively. |
| 2026-07-09 | Windows installed-app inventory for `orphans` (#7) | `reg.exe query` over the HKCU / HKLM / WOW6432Node `Uninstall` registry roots (Windows built-in; key layout from Microsoft's Installer docs) | — (OS tool, facts from vendor docs) | A | Wrapped as a subprocess in the deleted `src/orphan.rs`, fixtures under `testdata/windows/registry/`. Dropped, not moved, in #18. Recorded retroactively. |
| 2026-07-10 | Windows per-app network attribution for `net` (#8) | `windows-sys` crate — IP Helper `GetExtendedTcpTable` / `GetExtendedUdpTable`; `netstat -ano` + `tasklist` fallback | MIT / Apache-2.0 (`windows-sys`); OS tools | A | Linked dependency, the crate's only user was the deleted `src/net.rs`; dependency dropped with it in #18. Recorded retroactively. |
| 2026-07-12 | Windows Cloud Files eviction for `evict` (#10) | Cloud Files attributes via `MetadataExt::file_attributes()` (std); eviction via `attrib.exe +U -P` (Windows built-in; flag semantics from Microsoft's `attrib` docs) | — (OS tool) | A | Wrapped as a subprocess in the deleted `src/evict.rs`. Dropped, not moved, in #18. Recorded retroactively; the PR merged 2026-07-12. |
| 2026-08-08 | Eight capabilities relocated to the engine (`net` `orphans` `evict` `dupes` `slim-check` `sentinel` `photos` `rules`) | — (internal move, `3633c19`) | FSL-1.1-ALv2 → FSL-1.1-ALv2 | — | Not a new capability, no new third-party source, and **no licence change**: the conductor's own implementations were deleted and these commands now forward to `caezium/burrow-engine`, which carries the same licence and the same copyright holder. Windows implementations of `net`/`orphans`/`evict`/`dupes` were dropped rather than moved (see `STATUS.md`). Full per-capability record — including what is *not* established — in [the migration record](#the-2026-08-08-engine-migration) below. |

## The 2026-08-08 engine migration

`b5768ad` repointed this CLI onto the Rust engine and `3633c19` deleted its own copies of eight
commands (both squash-merged as #18, `8cff30f` on `main`). Because that moves code between repositories, the licence question has to be answered
here rather than assumed.

**The engine repository is `caezium/burrow-engine`**, born at `eb833f1` (2026-07-12). The eight
ports below are on its `main`; the `rules`/`sentinel` port and the platform guards this repo now
relies on (`e1a73da`, `5e42bf0`, `0dd5997`) are on `feat/contract-conformance` and **not yet
merged to `main`**, so an audit that reads only `main` will not find `rules` or `sentinel` there.

**It is FSL-1.1-ALv2, Copyright 2026 caezium — the same licence and the same holder as this
repo.** Verified by reading `LICENSE.md` at every ref in the repository, local and remote,
including `main`: all identical, and the file has exactly one commit in its history (the birth
commit). The engine has never been MIT. Anything in this project's docs calling it "the MIT
engine" is confusing it with `caezium/burrow-digger`, the archived MIT bash+Go mo fork that
carried the `burrow-engine` name until `32ec0f0`.

**So: no FSL code entered an MIT repository, because no MIT engine exists.** No relicensing
grant was needed for this direction and none was looked for; both sides are one licensor's work
under one licence. The MIT question in this migration runs the *other* way — see the open items.

Every capability below was transferred directly from this repository to the engine. This describes the repository transfer; each capability retains the original source mode recorded above, including Mode C work.

| Capability | Engine module | Route | Evidence |
|---|---|---|---|
| `net` | `src/net/mod.rs` | ported, macOS half only | `7328315` (#36) — "ported verbatim from burrow-cli's net.rs"; also names itself "the first port of a burrow-cli (not digger) command" |
| `orphans` | `src/orphan/mod.rs` | ported, macOS half only | `08906c3` (#37) — "Zero-dep port of burrow-cli's orphans command (macOS path)"; 154 substantive lines shared with the deleted `src/orphan.rs` |
| `slim-check` | `src/macho/mod.rs` | ported | `698bef9` (#38); 104 of the 111 substantive lines of this repo's `src/macho.rs` appear in it |
| `evict` | `src/evict/mod.rs` | ported, macOS half only | `9cce5c9` (#39) — "Zero-dep port of burrow-cli's macOS evict command" |
| `dupes` | `src/dupes/mod.rs` | ported | `9246e46` (#41) — "Ports burrow-cli's dupes command"; 159 substantive lines shared with the deleted `src/dupes.rs` |
| `photos` | `src/photos/mod.rs` | ported | `22855c2` (#42) + `130b3aa` (#43) — "byte-for-byte the same pipeline as burrow-cli, so hashes match the cli exactly" |
| `rules` | `src/rules/mod.rs` | ported | `e1a73da` — "RULES and SENTINEL are ported" (branch only) |
| `sentinel` | `src/sentinel/mod.rs` | ported | `e1a73da`, same commit (branch only) |

Direction is not in doubt: the engine's ports landed 2026-07-14/15, a month before this repo
deleted its originals on 2026-08-08, and each commit names `burrow-cli` as the source. The
line-overlap figures are corroboration measured for this record (identical substantive source
lines, whitespace-normalised), not the primary evidence.

Nothing with a copyleft obligation moved. All eight trace to Mode A/B/C rows above over
MIT/Apache-2.0/clean-room sources; the Commons-Clause and GPL/AGPL entries are ideas-only
(Mode C) and contributed no source in either repo.

### Current attribution and capability status

The historical review found missing engine attribution and a branch gap for `rules`,
`sentinel`, and platform guards. Those observations described the earlier engine tree.
The reviewed public engine includes those capabilities and has restored Windows `net` through
IP Helper with a `netstat` fallback; both sources report connection counts rather than bandwidth.
It carries
[`THIRD-PARTY-NOTICES.md`](https://github.com/caezium/burrow-engine/blob/main/THIRD-PARTY-NOTICES.md)
with the preserved upstream notices for its adapted code. Both current Rust projects retain
FSL-1.1-ALv2; the MIT terms remain attached to their upstream portions.

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
- Keep-newest-version heuristics (Device Support, doc caches) — concept from DevCleaner (GPL).

(Orphan confidence tiers, the declarative rule format, and the snapshot tree-diff have shipped
and moved into the table above.)
