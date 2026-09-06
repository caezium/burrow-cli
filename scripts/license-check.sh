#!/usr/bin/env bash
#
# license-check.sh — fail if any forbidden (copyleft / non-commercial) license appears in
# Burrow CLI's distributed dependencies. Burrow CLI is FSL-licensed; it must never ship
# GPL/AGPL/SSPL code. Our OWN license (FSL, in ./LICENSE.md) is intentionally exempt.
#
# Runs in CI and locally:  ./scripts/license-check.sh
set -euo pipefail
cd "$(dirname "$0")/.."

fail=0

echo "== scanning vendored third-party LICENSE / COPYING files =="
while IFS= read -r f; do
  case "$f" in
    ./LICENSE*|./THIRD-PARTY-LICENSES.md|./NOTICE) continue ;;  # our own files
  esac
  if grep -qiE "GNU (Lesser |Affero )?General Public License|Affero General Public|Server Side Public License|Commons Clause|for non-commercial" "$f"; then
    echo "  FORBIDDEN license text in: $f"
    fail=1
  fi
done < <(find . \( -path ./.git -o -path ./scripts \) -prune -o \( -iname 'LICENSE*' -o -iname 'COPYING*' \) -type f -print 2>/dev/null)

echo "== scanning Go dependency licenses (if applicable) =="
if [ -f go.mod ] && command -v go-licenses >/dev/null 2>&1; then
  if ! go-licenses check ./... --disallowed_types=forbidden,restricted; then
    echo "  go-licenses flagged restricted/forbidden dependencies"
    fail=1
  fi
else
  echo "  (skipped: no go.mod or go-licenses not installed)"
fi

# Rust (cargo-deny) hook — enabled once/if the conductor is Rust.
if [ -f deny.toml ] && command -v cargo-deny >/dev/null 2>&1; then
  echo "== cargo-deny =="
  cargo deny check licenses || fail=1
fi

if [ "$fail" -ne 0 ]; then
  echo ""
  echo "LICENSE CHECK FAILED — copyleft/forbidden license detected in dependencies."
  echo "Burrow CLI must only ship MIT/Apache deps. Reimplement clean-room instead (see PROVENANCE.md)."
  exit 1
fi
echo "license check passed."
