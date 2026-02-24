cask "openmango" do
  arch arm: "arm64", intel: "x86_64"

  version "0.1.7"
  # TODO: Compute SHA256 from release zips before submitting to homebrew-cask
  # sha256 arm: "...", intel: "..."

  url "https://github.com/ggagosh/openmango/releases/download/v#{version}/OpenMango-#{version}-macos-#{arch}.zip"
  name "OpenMango"
  desc "GPU-accelerated MongoDB client"
  homepage "https://openmango.app"

  depends_on macos: ">= :ventura"

  app "OpenMango.app"

  zap trash: [
    "~/Library/Application Support/OpenMango",
    "~/Library/Preferences/app.openmango.plist",
    "~/Library/Caches/OpenMango",
  ]
end
