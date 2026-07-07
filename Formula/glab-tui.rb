class GlabTui < Formula
  desc "Terminal user interface for GitLab and GitHub"
  homepage "https://github.com/rcieri/glab-tui"
  license "MIT"

  depends_on "gh"
  depends_on "glab" => :recommended

  on_macos do
    on_intel do
      url "https://github.com/rcieri/glab-tui/releases/download/v0.5.0/glab-tui-macos-amd64.tar.gz"
      sha256 "a502d7471e8e1dd54b847b4e170b546df6efb7d1798ee6958af9d7d4b3e93970"
    end
    on_arm do
      url "https://github.com/rcieri/glab-tui/releases/download/v0.5.0/glab-tui-macos-arm64.tar.gz"
      sha256 "9b8f94d6153b6e9dc52cd847cb9c026f45469a2b5aa72c863b4027a8b1672a55"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/rcieri/glab-tui/releases/download/v0.5.0/glab-tui-linux-amd64.tar.gz"
      sha256 "bbb2d52018c6627fea36571e65a7f2e1b684ab274abb58f977299af08ca2ca16"
    end
    on_arm do
      url "https://github.com/rcieri/glab-tui/releases/download/v0.5.0/glab-tui-linux-arm64.tar.gz"
      sha256 "a8851a94f602d19e14bf4a1eae0d8acd572a3ad2d858d102c3b1d5e21c981ab5"
    end
  end

  def install
    bin.install "glab-tui"
  end

  test do
    system "\#{bin}/glab-tui", "--help"
  end
end
