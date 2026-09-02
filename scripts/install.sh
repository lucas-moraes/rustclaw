#!/bin/bash
# RustClaw installer (opencode-style).
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/lucas-moraes/rustclaw/main/scripts/install.sh | bash
#
# Env options:
#   RUSTCLAW_VERSION=v0.2.0   pin a specific release tag
#   RUSTCLAW_INSTALL_DIR      install prefix (default: ~/.local/bin)

set -euo pipefail

REPO="lucas-moraes/rustclaw"
BIN="rustclaw"
VERSION="${RUSTCLAW_VERSION:-}"
INSTALL_DIR="${RUSTCLAW_INSTALL_DIR:-$HOME/.local/bin}"

log()  { printf '%s\n' "$*"; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

# Temp dir for downloads; set inside main(), cleaned on exit.
TMPDIR_INSTALL=""

detect_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Darwin) os="darwin" ;;
    Linux)  os="linux" ;;
    *) die "unsupported OS: $os (supported: macOS, Linux)" ;;
  esac
  case "$arch" in
    arm64|aarch64) [[ "$os" == "darwin" ]] && arch="arm64" || die "unsupported arch: $arch for $os" ;;
    x86_64|amd64)  arch="x64" ;;
    *) die "unsupported architecture: $arch (supported: macOS arm64, Linux x64)" ;;
  esac
  printf '%s-%s' "$os" "$arch"
}

download() {
  local url="$1" dest="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL -o "$dest" "$url" || return 1
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$dest" "$url" || return 1
  else
    die "need curl or wget to download"
  fi
}

main() {
  local target
  target="$(detect_target)"

  # Resolve the latest release tag when not pinned.
  if [ -z "$VERSION" ]; then
    local api_url="https://api.github.com/repos/$REPO/releases/latest"
    if command -v curl >/dev/null 2>&1; then
      VERSION="$(curl -fsSL "$api_url" | grep -o '"tag_name": *"[^"]*"' | sed 's/.*"tag_name": *"\(.*\)"/\1/')" || true
    elif command -v wget >/dev/null 2>&1; then
      VERSION="$(wget -qO- "$api_url" | grep -o '"tag_name": *"[^"]*"' | sed 's/.*"tag_name": *"\(.*\)"/\1/')" || true
    fi
    [ -n "$VERSION" ] || die "could not resolve latest release (set RUSTCLAW_VERSION to a tag, e.g. v0.2.0)"
  fi
  log "installing $BIN $VERSION for $target"

  local archive="rustclaw-${target}.tar.gz"
  local base="${RUSTCLAW_DOWNLOAD_BASE:-https://github.com/$REPO/releases/download/$VERSION}"
  TMPDIR_INSTALL="$(mktemp -d)"
  local tmp="$TMPDIR_INSTALL"
  trap 'rm -rf "$TMPDIR_INSTALL"' EXIT

  download "$base/$archive" "$tmp/$archive" \
    || die "download failed: $base/$archive (does the release exist?)"
  download "$base/checksums.txt" "$tmp/checksums.txt" \
    || log "warning: checksums.txt unavailable, skipping verification"

  if [ -f "$tmp/checksums.txt" ]; then
    local expected
    expected="$(grep "$archive" "$tmp/checksums.txt" | awk '{print $1}')"
    if [ -n "$expected" ]; then
      echo "$expected  $tmp/$archive" | shasum -a 256 -c - >/dev/null 2>&1 \
        || ( command -v sha256sum >/dev/null && echo "$expected  $tmp/$archive" | sha256sum -c - >/dev/null ) \
        || die "checksum mismatch for $archive"
      log "checksum ok"
    fi
  fi

  tar -xzf "$tmp/$archive" -C "$tmp"
  [ -f "$tmp/$BIN" ] || die "archive did not contain $BIN"
  chmod +x "$tmp/$BIN"

  mkdir -p "$INSTALL_DIR"
  mv "$tmp/$BIN" "$INSTALL_DIR/$BIN"

  # Ensure the install dir is on PATH (idempotent, no duplicates).
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
      export PATH="$INSTALL_DIR:$PATH"
      local rc_file
      rc_file="$HOME/.zshrc"
      [ "$SHELL" = "/bin/zsh" ] || rc_file="$HOME/.profile"
      {
        printf '\n# RustClaw (added by install script)\n'
        printf 'export PATH="%s:$PATH"\n' "$INSTALL_DIR"
      } >>"$rc_file"
      log "added $INSTALL_DIR to PATH in $rc_file (restart your shell)"
      ;;
  esac

  log ""
  log "✓ installed: $(command -v "$BIN" || printf '%s/%s' "$INSTALL_DIR" "$BIN")"
  log "  run 'rustclaw --version' to confirm"
}

main "$@"
