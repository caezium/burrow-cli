# Homebrew formula for Burrow CLI (source build via cargo).
# Tap: caezium/burrow (or homebrew-core once eligible).
#
# Note: Burrow CLI is FSL-1.1 (source-available, not OSI). homebrew-core requires an OSI
# license, so this ships via a custom tap until the FSL release converts to Apache-2.0
# (2 years after each release).
class BurrowCli < Formula
  desc "Agent-native macOS system-cleaning CLI (conductor over burrow-engine + fclones)"
  homepage "https://github.com/caezium/burrow-cli"
  license "LicenseRef-FSL-1.1-ALv2"
  head "https://github.com/caezium/burrow-cli.git", branch: "windows-legacy"

  depends_on "rust" => :build
  # Runtime engines are resolved at run time ($BURROW_ENGINE_DIR / $BURROW_FCLONES or PATH):
  depends_on "fclones" => :recommended

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "burrow", shell_output("#{bin}/burrow version")
    # dupes against an empty dir should emit a valid envelope and exit cleanly
    require "json"
    out = shell_output("#{bin}/burrow rules validate #{testpath} 2>/dev/null", 0)
    JSON.parse(out)
  end
end
