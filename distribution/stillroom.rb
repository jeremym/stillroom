# Homebrew cask for jeremym/homebrew-tap (Casks/stillroom.rb).
# scripts/update-tap.sh rewrites the `version` and `sha256` lines; keep their shape.
cask "stillroom" do
  version "0.1.0"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"

  url "https://github.com/jeremym/stillroom/releases/download/v#{version}/Stillroom-#{version}.zip"
  name "Stillroom"
  desc "Goal-first, local-first AI workflow studio over Ollama"
  homepage "https://github.com/jeremym/stillroom"

  livecheck do
    url :url
    strategy :github_latest
  end

  auto_updates false
  depends_on arch: :arm64
  depends_on macos: :ventura

  app "Stillroom.app"

  # Not notarized (no Apple Developer ID, by choice). Drop the quarantine flag Homebrew sets so
  # first launch isn't blocked. Integrity: the sha256 above + the certificate leaf hash in the
  # release notes (codesign -d -r- /Applications/Stillroom.app).
  postflight_steps do
    run "/usr/bin/xattr", args: ["-dr", "com.apple.quarantine", "{{appdir}}/Stillroom.app"], must_succeed: false
  end

  uninstall quit: "com.jeremym.stillroom"

  zap trash: [
    "~/Library/Application Support/com.jeremym.stillroom",
    "~/Library/Caches/com.jeremym.stillroom",
    "~/Library/Preferences/com.jeremym.stillroom.plist",
    "~/Library/Saved Application State/com.jeremym.stillroom.savedState",
    "~/Library/WebKit/com.jeremym.stillroom",
  ]

  caveats <<~EOS
    Stillroom is signed with the project's own certificate, not an Apple Developer ID,
    and is not notarized. The quarantine flag is removed on install so it launches
    without a Gatekeeper prompt.

    Models run through Ollama on this Mac. If you do not have it yet:
      brew install ollama && brew services start ollama
    Stillroom's first screen pulls a starter model, or pull your own: ollama pull qwen3:8b

    Your recipes, runs and results live in ~/Library/Application Support/com.jeremym.stillroom.
    Any API key you add stays in your login keychain (service com.jeremym.stillroom) after
    uninstall; delete it in Keychain Access if you want it gone.
  EOS
end
