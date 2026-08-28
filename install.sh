#!/usr/bin/env sh
# Installs the latest tt (timetracker-rs) release for Linux/macOS.
#
#   curl -fsSL https://raw.githubusercontent.com/linus-skold/timetracker-rs/main/install.sh | sh
#
# Override the install directory with TT_INSTALL_DIR (defaults to
# ~/.local/bin, created if missing).

set -eu

REPO="linus-skold/timetracker-rs"
INSTALL_DIR="${TT_INSTALL_DIR:-$HOME/.local/bin}"

os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
  Linux) os_part="unknown-linux-gnu" ;;
  Darwin) os_part="apple-darwin" ;;
  *)
    echo "error: unsupported OS: $os" >&2
    exit 1
    ;;
esac

case "$arch" in
  x86_64|amd64) arch_part="x86_64" ;;
  arm64|aarch64)
    if [ "$os" = "Linux" ]; then
      echo "error: no prebuilt tt binary for Linux/$arch yet" >&2
      exit 1
    fi
    arch_part="aarch64"
    ;;
  *)
    echo "error: unsupported architecture: $arch" >&2
    exit 1
    ;;
esac

target="${arch_part}-${os_part}"
asset="tt-${target}"
url="https://github.com/${REPO}/releases/latest/download/${asset}"

mkdir -p "$INSTALL_DIR"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

echo "Downloading tt for ${target}..."
if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$url" -o "$tmp"
elif command -v wget >/dev/null 2>&1; then
  wget -q "$url" -O "$tmp"
else
  echo "error: need curl or wget to install" >&2
  exit 1
fi

chmod +x "$tmp"
mv "$tmp" "$INSTALL_DIR/tt"
trap - EXIT

echo "Installed tt to $INSTALL_DIR/tt"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo
    echo "warning: $INSTALL_DIR is not on your PATH."
    echo "Add this to your shell profile:"
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
    ;;
esac

# Mirrors the per-shell hint table in src/commands.rs (`completions`).
if "$INSTALL_DIR/tt" completions --help >/dev/null 2>&1; then
  echo
  echo "Shell completion is available. To enable it, run:"
  case "$(basename "${SHELL:-}")" in
    zsh)    echo "  echo 'eval \"\$(tt completions zsh)\"' >> ~/.zshrc" ;;
    bash)   echo "  echo 'eval \"\$(tt completions bash)\"' >> ~/.bashrc" ;;
    fish)   echo "  echo 'tt completions fish | source' >> ~/.config/fish/config.fish" ;;
    elvish) echo "  echo 'eval (tt completions elvish | slurp)' >> ~/.config/elvish/rc.elv" ;;
    nu)     echo "  tt completions nu | save -f (\$nu.user-autoload-dirs.0 | path join tt-completer.nu)" ;;
    *)      echo "  tt completions --help   (see docs/usage.md#tt-completions-shell)" ;;
  esac
fi

"$INSTALL_DIR/tt" --version || true
