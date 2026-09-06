# Third-party licenses

Burrow CLI remains FSL-1.1-ALv2; see `LICENSE.md`. This file distinguishes the crates linked
into `burrow` from independently licensed programs invoked as subprocesses. Public source
availability does not change the licenses of this project or its dependencies.

## Linked Rust crates

The direct dependencies declared in `Cargo.toml` are:

| Crate | License | Source |
|---|---|---|
| serde | MIT OR Apache-2.0 | <https://github.com/serde-rs/serde> |
| serde_json | MIT OR Apache-2.0 | <https://github.com/serde-rs/json> |
| trash | MIT | <https://github.com/Byron/trash-rs> |
| image | MIT OR Apache-2.0 | <https://github.com/image-rs/image> |

`Cargo.lock` pins the complete dependency graph. The full original license and notice files
for every resolved dependency, including transitive, build and target-specific packages, are
included under [`LICENSES/cargo/`](LICENSES/cargo/). The complete
[package inventory](LICENSES/cargo-packages.json) records versions, upstream license expressions,
source repositories and SHA-256 hashes for each notice. [LICENSES/README.md](LICENSES/README.md)
describes the audited upstream exceptions and regeneration procedure.

`cargo deny check licenses` checks the dependency policy. `scripts/license-check.sh` also
verifies that the bundled notice inventory matches the locked graph and original source files.
Windows CI artifacts and the Homebrew package install `LICENSE.md`, `NOTICE`, this file, and
the complete `LICENSES/` tree alongside the binary or in its package share directory.

These files cover the Cargo dependency graph. Binary distributors must additionally preserve
the applicable Rust toolchain/standard-library notices and notices for separately bundled
engines or tools; those components are not vendored in this source snapshot.

## Separate-process runtimes

### burrow-digger / legacy Mole adapter — MIT

This branch retains the Bash/Go and PowerShell adapter contract. The Bash/Go fork formerly
named `burrow-engine` now lives at `caezium/burrow-digger`; the current Rust repository using
that name is separately FSL-licensed. Burrow's Windows app supplies the PowerShell runtime
and retains that runtime's own MIT license and pinned provenance in its `Assets/Mole` tree.

Fork of [tw93/Mole](https://github.com/tw93/Mole) at commit `9daf936` (V1.42.0), the last
MIT-licensed version before its 2026-06-11 relicense to GPL-3.0. Original copyright preserved:

```
MIT License

Copyright (c) 2025 tw93

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

Modifications in the fork are © 2026 caezium, released under the same MIT terms.

---

### fclones — MIT

<https://github.com/pkolaczk/fclones> — Copyright (c) Piotr Kołaczkowski. MIT License.
No binary is vendored in this source snapshot. A binary distributor must include the upstream license alongside that component.

---

### czkawka_core / czkawka_cli — MIT

<https://github.com/qarmin/czkawka> — Copyright (c) Rafał Mikrut. MIT License.
**Only** the MIT-licensed `czkawka_core`/`czkawka_cli` are used. The Krokiet and Cedinia
frontends are GPL-3.0 and are **not** used or distributed.
No binary is vendored in this source snapshot. A binary distributor must include the upstream license alongside that component.

---

### Bulk Crap Uninstaller — Apache-2.0 *(Windows)*

<https://github.com/Klocman/Bulk-Crap-Uninstaller> — Copyright Marcin Szeniak. Apache-2.0.
No binary is vendored in this source snapshot. A binary distributor must include the upstream license and NOTICE alongside that component.
