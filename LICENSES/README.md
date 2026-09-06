# Locked Cargo dependency notices

`cargo-packages.json` inventories every dependency resolved by `cargo metadata --locked`
without a platform filter. It includes target-only and build dependencies so the same notice
payload can accompany Linux, macOS and Windows builds. Each `texts[].file` is relative to the
repository/package root; each `sha256` identifies the exact original bytes. Local Cargo cache
paths and credentials are never included.

Original package-root LICENSE, LICENCE, COPYING, COPYRIGHT, NOTICE and UNLICENSE files, plus
any declared `license_file`, are copied into `cargo/<name>-<version>/` without editing. All
alternative license texts supplied by a package are retained; their presence does not change
the upstream SPDX expression or impose all alternatives simultaneously.

## objc2 archive exceptions

The locked objc2, objc2-encode and objc2-foundation archives declare MIT but omit license files.
For each, the current upstream `LICENSE.md` is preserved from the exact revision recorded in
that crate's `.cargo_vcs_info.json`. It discusses the project's license terms and Apple SDK
context, and links the MIT text rather than reproducing it.

The original full MIT notice, copyright Steven Sheldon, is also preserved from upstream
revision `9961247c1a82027d6edbe6c516011b1363b9354c` as `LICENSE-MIT-original.txt`. Upstream later
replaced that file with its explanatory `LICENSE.md`; retaining both keeps the original
copyright/permission notice alongside the current explanation. Their pinned source URLs and
hashes are recorded in the inventory. No license grant or copyright holder was invented.

## Updating and verification

After a dependency update, run `cargo fetch --locked` and
`python3 scripts/cargo-notices.py`, then review the new inventory and upstream texts.
`python3 scripts/cargo-notices.py --check` verifies the graph and every bundled text without
modifying files or fetching supplemental upstream notices. New archive omissions require an
explicit pinned-source audit; the collector fails if it cannot find a license text.

Distribute this whole directory with `LICENSE.md`, `NOTICE` and `THIRD-PARTY-LICENSES.md`.
This Cargo inventory does not replace notices required for Rust's toolchain/standard library
or separately packaged engines and external executables.
