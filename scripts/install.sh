#!/bin/sh
# fbsy installer for Linux and macOS.
#
#   curl -fsSL https://raw.githubusercontent.com/anonto42/fbsy/main/scripts/install.sh | sh
#
# Detects your OS/arch, downloads the matching release binary, verifies its
# checksum, clears the macOS quarantine flag, installs it to ~/.local/bin, and
# runs `fbsy install` to set up PATH and data directories.
#
# Environment overrides:
#   FBSY_VERSION=0.2.0       install a specific version instead of latest
#   FBSY_INSTALL_DIR=DIR     install the binary here (default: ~/.local/bin)
#   FBSY_NO_VERIFY=1         skip checksum verification
set -eu

REPO="anonto42/fbsy"

red()   { printf '\033[0;31m%s\033[0m\n' "$1" >&2; }
green() { printf '\033[0;32m%s\033[0m\n' "$1"; }
info()  { printf '\033[1;33m%s\033[0m\n' "$1"; }
die()   { red "error: $1"; exit 1; }

# ── 1. Detect platform → release asset name ──────────────────────────────────
os="$(uname -s)"
arch="$(uname -m)"
asset=""
case "$os" in
  Linux)
    case "$arch" in
      x86_64|amd64)        asset="fbsy-linux-x86_64" ;;
      aarch64|arm64)       asset="fbsy-linux-aarch64" ;;
      *) die "unsupported Linux architecture: $arch" ;;
    esac
    ;;
  Darwin)
    case "$arch" in
      x86_64)              asset="fbsy-macos-intel" ;;
      arm64)               asset="fbsy-macos-arm64" ;;
      *) die "unsupported macOS architecture: $arch" ;;
    esac
    ;;
  *)
    die "unsupported OS: $os (this script handles Linux and macOS; use install.ps1 on Windows)"
    ;;
esac

# ── 2. Resolve download URL base ─────────────────────────────────────────────
if [ -n "${FBSY_VERSION:-}" ]; then
  base="https://github.com/$REPO/releases/download/v${FBSY_VERSION}"
else
  base="https://github.com/$REPO/releases/latest/download"
fi

# ── helpers: download to stdout-file ─────────────────────────────────────────
download() {
  # download <url> <dest>
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$1" -o "$2"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$2" "$1"
  else
    die "neither curl nor wget is available"
  fi
}

tmp="$(mktemp -d "${TMPDIR:-/tmp}/fbsy-install.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
bin_tmp="$tmp/fbsy"

# ── 3. Download the binary ───────────────────────────────────────────────────
info "Downloading $asset ..."
download "$base/$asset" "$bin_tmp" || die "download failed: $base/$asset"

# ── 4. Verify checksum (best effort) ─────────────────────────────────────────
if [ "${FBSY_NO_VERIFY:-0}" != "1" ]; then
  sums_tmp="$tmp/checksums.txt"
  if download "$base/checksums.txt" "$sums_tmp" 2>/dev/null; then
    expected="$(grep " $asset\$" "$sums_tmp" 2>/dev/null | awk '{print $1}' | head -n1)"
    if [ -n "$expected" ]; then
      if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "$bin_tmp" | awk '{print $1}')"
      elif command -v shasum >/dev/null 2>&1; then
        actual="$(shasum -a 256 "$bin_tmp" | awk '{print $1}')"
      else
        actual=""
      fi
      if [ -n "$actual" ] && [ "$actual" != "$expected" ]; then
        die "checksum mismatch for $asset (expected $expected, got $actual)"
      fi
      [ -n "$actual" ] && green "Checksum verified."
    else
      info "No checksum entry for $asset; skipping verification."
    fi
  else
    info "checksums.txt not available; skipping verification."
  fi
fi

# ── 5. Make executable; clear macOS quarantine ───────────────────────────────
chmod +x "$bin_tmp"
if [ "$os" = "Darwin" ]; then
  xattr -d com.apple.quarantine "$bin_tmp" 2>/dev/null || true
fi

# ── 6. Install to bin dir ────────────────────────────────────────────────────
install_dir="${FBSY_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$install_dir"
mv -f "$bin_tmp" "$install_dir/fbsy"
green "Installed fbsy to $install_dir/fbsy"

# ── 7. Finish setup (PATH + data dirs) via the binary itself ─────────────────
# `fbsy install` prints its own "open a new shell" guidance, so the script does
# not repeat it on success.
if ! "$install_dir/fbsy" install; then
  echo
  info "Installed the binary, but setup did not finish."
  info "Run '$install_dir/fbsy install' manually, then open a new shell."
fi
