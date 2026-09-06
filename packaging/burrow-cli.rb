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
  head "https://github.com/caezium/burrow-cli.git", branch: "main"

  depends_on "rust" => :build
  # Runtime engines are resolved at run time ($BURROW_ENGINE_DIR / $BURROW_FCLONES or PATH):
  depends_on "fclones" => :recommended

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "burrow", shell_output("#{bin}/burrow version")
    # A conductor-native preview works without installing a separate engine.
    require "json"
    fixture = testpath/"preview.txt"
    fixture.write "keep me"
    ENV["BURROW_TELEMETRY"] = "0"
    out = shell_output("#{bin}/burrow trash #{fixture}", 0)
    assert_equal true, JSON.parse(out).fetch("ok")
    assert_equal "keep me", fixture.read
  end
end
