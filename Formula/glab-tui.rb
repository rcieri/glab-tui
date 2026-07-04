class GlabTui < Formula
  desc "Terminal user interface for GitLab and GitHub"
  homepage "https://github.com/rcieri/glab-tui"
  license "MIT"

  on_macos do
    on_intel do
      url "https://github.com/rcieri/glab-tui/releases/download/v2.3.0/glab-tui-macos-amd64.tar.gz"
      sha256 "2e23aae181debb2078d7ed6e79bc0e2f6d4da8ced6c2e229f8f53fb350cb2304"
    end
    on_arm do
      url "https://github.com/rcieri/glab-tui/releases/download/v2.3.0/glab-tui-macos-arm64.tar.gz"
      sha256 "e3f482a43bf75736afd49261aa6bc6fbcb333bd2bd633814169087b75b4f2314"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/rcieri/glab-tui/releases/download/v2.3.0/glab-tui-linux-amd64.tar.gz"
      sha256 "8f067e1996079230067beb9e66542e3ca0c57ee0d5b06dacd74befadff64cfe9"
    end
    on_arm do
      url "https://github.com/rcieri/glab-tui/releases/download/v2.3.0/glab-tui-linux-arm64.tar.gz"
      sha256 "efb51f6cd519b8f7b48d960ac862bf12af98b0d0f76c351972649f3f3e705c95"
    end
  end

  depends_on "gh"
  depends_on "glab" => :recommended

  def install
    bin.install "glab-tui"
  end

  test do
    system "#{bin}/glab-tui", "--help"
  end
end
