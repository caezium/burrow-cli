#!/usr/bin/env bash
#
# release.sh — build + sign the Burrow CLI release binary.
#
# Signing tiers:
#   - ad-hoc (default here): `codesign -s -`  — no certificate required, works everywhere,
#     but does not survive Gatekeeper on other machines.
#   - Developer ID (the real cross-machine fix): requires a paid Apple Developer ID cert +
#     `xcrun notarytool`; configure your own credentials for the commands below.
set -euo pipefail
cd "$(dirname "$0")/.."

# Telemetry capture key (compiled in via `option_env!`, see src/telemetry.rs). The environment
# wins — CI sets BURROW_POSTHOG_KEY as a secret — and the gitignored `.release.env` is only a
# local convenience for the same variables (template: `.release.env.example`). Neither is
# required: a build without a key simply ships with telemetry off.
if [ -z "${BURROW_POSTHOG_KEY:-}" ] && [ -f .release.env ]; then
  # shellcheck disable=SC1091
  . ./.release.env
fi
if [ -z "${BURROW_POSTHOG_KEY:-}" ]; then
  echo "note: BURROW_POSTHOG_KEY not set — building with telemetry off" >&2
fi

echo "== build (release) =="
cargo build --release
BIN="target/release/burrow"

echo "== ad-hoc codesign =="
codesign --force --sign - --timestamp=none "$BIN"
codesign --verify --verbose "$BIN"
echo "ad-hoc signed: $BIN"

# --- Developer ID (gated: needs a cert + notarytool credentials) ---
# IDENTITY="Developer ID Application: <Your Name> (<TEAMID>)"
# codesign --force --options runtime --timestamp --sign "$IDENTITY" "$BIN"
# ditto -c -k --keepParent "$BIN" burrow.zip
# xcrun notarytool submit burrow.zip --keychain-profile burrow-notary --wait
# xcrun stapler staple "$BIN"

echo "== license gate before publish =="
./scripts/license-check.sh
