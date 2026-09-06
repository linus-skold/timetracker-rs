#!/usr/bin/env sh
# Installs the latest tt (timetracker-rs) release for Linux/macOS/Git Bash on Windows x86_64.
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
bin_name="tt"

case "$os" in
  Linux) os_part="unknown-linux-gnu" ;;
  Darwin) os_part="apple-darwin" ;;
  MINGW*|MSYS*|CYGWIN*)
    os_part="pc-windows-msvc"
    bin_name="tt.exe"
    ;;
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
    if [ "$os_part" = "pc-windows-msvc" ]; then
      echo "error: no prebuilt tt binary for Windows/$arch yet" >&2
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
if [ "$os_part" = "pc-windows-msvc" ]; then
  asset="${asset}.exe"
fi
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
mv "$tmp" "$INSTALL_DIR/$bin_name"
trap - EXIT

echo "Installed tt to $INSTALL_DIR/$bin_name"

if [ "$os_part" = "pc-windows-msvc" ]; then
  bashrc="$HOME/.bashrc"
  case "$INSTALL_DIR" in
    "$HOME")
      path_export='export PATH="$HOME:$PATH"'
      ;;
    "$HOME"/*)
      install_dir_suffix="${INSTALL_DIR#"$HOME"/}"
      path_export="export PATH=\"\$HOME/$install_dir_suffix:\$PATH\""
      ;;
    *)
      path_export="export PATH=\"$INSTALL_DIR:\$PATH\""
      ;;
  esac
  bashrc_updated=0

  if [ -f "$bashrc" ]; then
    if ! grep -F "$path_export" "$bashrc" >/dev/null 2>&1; then
      if printf '\n%s\n' "$path_export" >> "$bashrc"; then
        bashrc_updated=1
      fi
    fi
  else
    if printf '%s\n' "$path_export" > "$bashrc"; then
      bashrc_updated=1
    fi
  fi

  if [ "$bashrc_updated" -eq 1 ]; then
    echo "Added $INSTALL_DIR to $bashrc."
    echo "Run: source $bashrc"
  fi
fi

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
if "$INSTALL_DIR/$bin_name" completions --help >/dev/null 2>&1; then
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

"$INSTALL_DIR/$bin_name" --version || true
